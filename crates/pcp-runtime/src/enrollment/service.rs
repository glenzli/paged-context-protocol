use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use pcp_client::{AccessMode, EmbeddedPcpClient};
use pcp_core::{AccessPrincipal, AccessPrincipalType, AccessSession, CreateScopeRequest};
use pcp_rpc::{
    BeginEnrollmentParams, ENROLLMENT_ADMIN_REQUEST_SCHEMA, ENROLLMENT_MAX_RESPONSE_FRAME_BYTES,
    ENROLLMENT_REQUEST_SCHEMA, EmptyParams, EnrollmentAdminRequest, EnrollmentAdminResponse,
    EnrollmentAdminResult, EnrollmentError, EnrollmentRegistrationIdParams, EnrollmentRequest,
    EnrollmentRequestIdParams, EnrollmentResponse, EnrollmentResult, EnrollmentServiceIdentity,
    EnrollmentSession, EnrollmentStatusParams, OpenEnrollmentSessionParams, PendingEnrollmentView,
    RegisteredClientView, RequestedAccessMode, RunningRuntimeEndpoint,
};
use pcp_store::PcpStore;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    config::EnrollmentConfig,
    state::{EnrollmentState, StateFile, StoredDecision, StoredRegistration, StoredRequest},
    transport::AdminServer,
};
use crate::infra_socket::BoundInfraSocket;

const LOCAL_UNIX_SOCKET_BINDING: &str = "infra.local.unix-socket";
const MAX_PENDING_REQUESTS: usize = 16;
const MAX_ACTIVE_REGISTRATIONS: usize = 32;

pub struct EnrollmentManager {
    handler: EnrollmentHandler,
    admin: AdminServer,
}

impl EnrollmentManager {
    pub async fn start(
        config: EnrollmentConfig,
        store: Arc<dyn PcpStore>,
        instance_id: String,
        generation: String,
    ) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        let (state_file, mut state) = StateFile::load(config.state_path)?;
        validate_loaded_state(&state)?;
        cleanup_expired_requests(&mut state);
        state_file.write(&state)?;
        let handler = EnrollmentHandler {
            inner: Arc::new(EnrollmentInner {
                store,
                identity_id: instance_id.clone(),
                service: EnrollmentServiceIdentity {
                    kind: "pcp".to_owned(),
                    instance_id,
                    generation,
                },
                runtime_root: config.runtime_root,
                request_ttl: config.request_ttl,
                state_file,
                state: tokio::sync::Mutex::new(state),
                sessions: tokio::sync::Mutex::new(HashMap::new()),
            }),
        };
        let admin = AdminServer::start(config.admin_socket_path, handler.clone()).await?;
        Ok(Some(Self { handler, admin }))
    }

    pub(crate) fn handler(&self) -> EnrollmentHandler {
        self.handler.clone()
    }

    pub fn admin_socket_path(&self) -> &Path {
        self.admin.socket_path()
    }

    pub fn is_finished(&self) -> bool {
        self.admin.is_finished()
    }

    pub async fn shutdown(&mut self) {
        self.admin.shutdown().await;
        let mut sessions = self.handler.inner.sessions.lock().await;
        sessions.clear();
    }
}

#[derive(Clone)]
pub(crate) struct EnrollmentHandler {
    inner: Arc<EnrollmentInner>,
}

struct EnrollmentInner {
    store: Arc<dyn PcpStore>,
    identity_id: String,
    service: EnrollmentServiceIdentity,
    runtime_root: PathBuf,
    request_ttl: Duration,
    state_file: StateFile,
    state: tokio::sync::Mutex<EnrollmentState>,
    sessions: tokio::sync::Mutex<HashMap<String, DynamicSession>>,
}

struct DynamicSession {
    wire: EnrollmentSession,
    _endpoint: RunningRuntimeEndpoint,
}

impl EnrollmentHandler {
    pub(crate) async fn handle_public(&self, request_bytes: &[u8]) -> Vec<u8> {
        let response = match self.dispatch_public(request_bytes).await {
            Ok(response) => encode(&response).unwrap_or_else(internal_public_error),
            Err(error) => encode(&EnrollmentError::public(error.code, error.message))
                .unwrap_or_else(internal_public_error),
        };
        if response.len().saturating_add(1) <= ENROLLMENT_MAX_RESPONSE_FRAME_BYTES {
            response
        } else {
            encode(&EnrollmentError::public(
                "response_too_large",
                "enrollment response exceeds the frame limit",
            ))
            .unwrap_or_else(internal_public_error)
        }
    }

    async fn dispatch_public(
        &self,
        request_bytes: &[u8],
    ) -> std::result::Result<EnrollmentResponse, ProtocolError> {
        let request: EnrollmentRequest = serde_json::from_slice(request_bytes)
            .map_err(|_| ProtocolError::invalid("request is not valid PCP enrollment JSON"))?;
        if request.schema != ENROLLMENT_REQUEST_SCHEMA {
            return Err(ProtocolError::invalid(
                "unsupported PCP enrollment request schema",
            ));
        }
        match request.operation.as_str() {
            "begin" => {
                let params = request
                    .decode_params::<BeginEnrollmentParams>("begin")
                    .map_err(|_| ProtocolError::invalid("invalid begin params"))?;
                self.begin(params).await
            }
            "status" => {
                let params = request
                    .decode_params::<EnrollmentStatusParams>("status")
                    .map_err(|_| ProtocolError::invalid("invalid status params"))?;
                self.status(params).await
            }
            "open_session" => {
                let params = request
                    .decode_params::<OpenEnrollmentSessionParams>("open_session")
                    .map_err(|_| ProtocolError::invalid("invalid open_session params"))?;
                self.open_session(params).await
            }
            _ => Err(ProtocolError::invalid(
                "unsupported PCP enrollment operation",
            )),
        }
    }

    async fn begin(
        &self,
        params: BeginEnrollmentParams,
    ) -> std::result::Result<EnrollmentResponse, ProtocolError> {
        validate_begin(&params)?;
        let credential_hash = hash_credential(&params.credential)?;
        let now = Utc::now();
        let mut state = self.inner.state.lock().await;
        let cleaned = cleanup_expired_requests(&mut state);
        if let Some(existing) = state
            .requests
            .iter()
            .find(|request| {
                credential_matches(&request.credential_hash, &credential_hash)
                    && request.client == params.client
                    && request.requested_access == params.requested_access
            })
            .cloned()
        {
            if cleaned {
                self.inner
                    .state_file
                    .write(&state)
                    .map_err(|_| ProtocolError::internal())?;
            }
            drop(state);
            let result = match &existing.decision {
                StoredDecision::Approved { registration_id } => EnrollmentResult::Active {
                    session: self
                        .open_registration(registration_id, &credential_hash)
                        .await?,
                },
                _ => result_for_request(&existing, &[], None)?,
            };
            return Ok(EnrollmentResponse::new("begin", result));
        }
        if state
            .requests
            .iter()
            .filter(|request| matches!(request.decision, StoredDecision::Pending))
            .count()
            >= MAX_PENDING_REQUESTS
        {
            return Err(ProtocolError::capacity());
        }
        let expires_at = now
            + chrono::Duration::from_std(self.inner.request_ttl)
                .map_err(|_| ProtocolError::internal())?;
        let request = StoredRequest {
            request_id: format!("req_{}", Uuid::new_v4().simple()),
            client: params.client,
            requested_access: params.requested_access,
            credential_hash,
            requested_at: now,
            expires_at,
            decision: StoredDecision::Pending,
        };
        let result = result_for_request(&request, &state.registrations, None)?;
        let mut next = state.clone();
        next.requests.push(request);
        self.inner
            .state_file
            .write(&next)
            .map_err(|_| ProtocolError::internal())?;
        *state = next;
        Ok(EnrollmentResponse::new("begin", result))
    }

    async fn status(
        &self,
        params: EnrollmentStatusParams,
    ) -> std::result::Result<EnrollmentResponse, ProtocolError> {
        let credential_hash = hash_credential(&params.credential)?;
        let request = {
            let mut state = self.inner.state.lock().await;
            let cleaned = cleanup_expired_requests(&mut state);
            if cleaned {
                self.inner
                    .state_file
                    .write(&state)
                    .map_err(|_| ProtocolError::internal())?;
            }
            state
                .requests
                .iter()
                .find(|request| {
                    request.request_id == params.request_id
                        && credential_matches(&request.credential_hash, &credential_hash)
                })
                .cloned()
                .ok_or_else(ProtocolError::not_found)?
        };
        let session = match &request.decision {
            StoredDecision::Approved { registration_id } => Some(
                self.open_registration(registration_id, &credential_hash)
                    .await?,
            ),
            _ => None,
        };
        let state = self.inner.state.lock().await;
        let result = result_for_request(&request, &state.registrations, session)?;
        Ok(EnrollmentResponse::new("status", result))
    }

    async fn open_session(
        &self,
        params: OpenEnrollmentSessionParams,
    ) -> std::result::Result<EnrollmentResponse, ProtocolError> {
        let credential_hash = hash_credential(&params.credential)?;
        let session = self
            .open_registration(&params.registration_id, &credential_hash)
            .await?;
        Ok(EnrollmentResponse::new(
            "open_session",
            EnrollmentResult::Active { session },
        ))
    }

    async fn open_registration(
        &self,
        registration_id: &str,
        credential_hash: &str,
    ) -> std::result::Result<EnrollmentSession, ProtocolError> {
        let registration = {
            let state = self.inner.state.lock().await;
            state
                .registrations
                .iter()
                .find(|registration| {
                    registration.registration_id == registration_id
                        && registration.revoked_at.is_none()
                        && credential_matches(&registration.credential_hash, credential_hash)
                })
                .cloned()
                .ok_or_else(ProtocolError::not_found)?
        };
        self.ensure_identity_scope(&registration.approved_access)
            .await?;

        let read_scopes = if registration.approved_access.read_all_scopes {
            self.inner
                .store
                .local_scope_names()
                .await
                .map_err(|_| ProtocolError::unavailable())?
        } else {
            Vec::new()
        };
        let access = access_session(
            &registration,
            &self.inner.identity_id,
            &self.inner.service.generation,
            read_scopes,
        )?;

        let mut sessions = self.inner.sessions.lock().await;
        if let Some(session) = sessions.get(registration_id)
            && !session._endpoint.is_finished()
            && session.wire.access == access
        {
            return Ok(session.wire.clone());
        }
        sessions.remove(registration_id);

        let bound = BoundInfraSocket::bind(&self.inner.runtime_root)
            .map_err(|_| ProtocolError::unavailable())?;
        let (endpoint, socket_path, listener) = bound.into_parts();
        let client = EmbeddedPcpClient::shared(Arc::clone(&self.inner.store), access.clone());
        let running = RunningRuntimeEndpoint::from_bound_listener(&socket_path, listener, client);
        let wire = EnrollmentSession {
            registration_id: registration.registration_id.clone(),
            service: self.inner.service.clone(),
            binding: LOCAL_UNIX_SOCKET_BINDING.to_owned(),
            endpoint,
            access,
        };
        sessions.insert(
            registration.registration_id.clone(),
            DynamicSession {
                wire: wire.clone(),
                _endpoint: running,
            },
        );
        drop(sessions);

        let mut state = self.inner.state.lock().await;
        let mut next = state.clone();
        let still_active = if let Some(stored) = next.registrations.iter_mut().find(|stored| {
            stored.registration_id == registration.registration_id && stored.revoked_at.is_none()
        }) {
            stored.last_opened_at = Some(Utc::now());
            self.inner
                .state_file
                .write(&next)
                .map_err(|_| ProtocolError::internal())?;
            *state = next;
            true
        } else {
            false
        };
        drop(state);
        if !still_active {
            self.inner.sessions.lock().await.remove(registration_id);
            return Err(ProtocolError::not_found());
        }
        Ok(wire)
    }

    pub(super) async fn handle_admin(&self, request_bytes: &[u8]) -> Vec<u8> {
        let response = match self.dispatch_admin(request_bytes).await {
            Ok(response) => encode(&response).unwrap_or_else(internal_admin_error),
            Err(error) => encode(&EnrollmentError::admin(error.code, error.message))
                .unwrap_or_else(internal_admin_error),
        };
        if response.len().saturating_add(1) <= ENROLLMENT_MAX_RESPONSE_FRAME_BYTES {
            response
        } else {
            encode(&EnrollmentError::admin(
                "response_too_large",
                "enrollment administration response exceeds the frame limit",
            ))
            .unwrap_or_else(internal_admin_error)
        }
    }

    async fn dispatch_admin(
        &self,
        request_bytes: &[u8],
    ) -> std::result::Result<EnrollmentAdminResponse, ProtocolError> {
        let request: EnrollmentAdminRequest =
            serde_json::from_slice(request_bytes).map_err(|_| {
                ProtocolError::invalid("request is not valid PCP enrollment admin JSON")
            })?;
        if request.schema != ENROLLMENT_ADMIN_REQUEST_SCHEMA {
            return Err(ProtocolError::invalid(
                "unsupported PCP enrollment admin request schema",
            ));
        }
        match request.operation.as_str() {
            "snapshot" => {
                request
                    .decode_params::<EmptyParams>("snapshot")
                    .map_err(|_| ProtocolError::invalid("invalid snapshot params"))?;
                self.admin_snapshot().await
            }
            "approve" => {
                let params = request
                    .decode_params::<EnrollmentRequestIdParams>("approve")
                    .map_err(|_| ProtocolError::invalid("invalid approve params"))?;
                self.approve(&params.request_id).await?;
                Ok(EnrollmentAdminResponse::new(
                    "approve",
                    EnrollmentAdminResult::Applied,
                ))
            }
            "reject" => {
                let params = request
                    .decode_params::<EnrollmentRequestIdParams>("reject")
                    .map_err(|_| ProtocolError::invalid("invalid reject params"))?;
                self.reject(&params.request_id).await?;
                Ok(EnrollmentAdminResponse::new(
                    "reject",
                    EnrollmentAdminResult::Applied,
                ))
            }
            "revoke" => {
                let params = request
                    .decode_params::<EnrollmentRegistrationIdParams>("revoke")
                    .map_err(|_| ProtocolError::invalid("invalid revoke params"))?;
                self.revoke(&params.registration_id).await?;
                Ok(EnrollmentAdminResponse::new(
                    "revoke",
                    EnrollmentAdminResult::Applied,
                ))
            }
            _ => Err(ProtocolError::invalid(
                "unsupported PCP enrollment admin operation",
            )),
        }
    }

    async fn admin_snapshot(&self) -> std::result::Result<EnrollmentAdminResponse, ProtocolError> {
        let mut state = self.inner.state.lock().await;
        if cleanup_expired_requests(&mut state) {
            self.inner
                .state_file
                .write(&state)
                .map_err(|_| ProtocolError::internal())?;
        }
        let pending = state
            .requests
            .iter()
            .filter(|request| matches!(request.decision, StoredDecision::Pending))
            .map(|request| PendingEnrollmentView {
                request_id: request.request_id.clone(),
                client: request.client.clone(),
                requested_access: request.requested_access.clone(),
                requested_at: format_time(request.requested_at),
                expires_at: format_time(request.expires_at),
            })
            .collect();
        let registrations = state
            .registrations
            .iter()
            .filter(|registration| registration.revoked_at.is_none())
            .map(|registration| RegisteredClientView {
                registration_id: registration.registration_id.clone(),
                client: registration.client.clone(),
                approved_access: registration.approved_access.clone(),
                created_at: format_time(registration.created_at),
                last_opened_at: registration.last_opened_at.map(format_time),
            })
            .collect();
        Ok(EnrollmentAdminResponse::new(
            "snapshot",
            EnrollmentAdminResult::Snapshot {
                pending,
                registrations,
            },
        ))
    }

    async fn approve(&self, request_id: &str) -> std::result::Result<(), ProtocolError> {
        let mut state = self.inner.state.lock().await;
        let mut next = state.clone();
        cleanup_expired_requests(&mut next);
        if next
            .registrations
            .iter()
            .filter(|registration| registration.revoked_at.is_none())
            .count()
            >= MAX_ACTIVE_REGISTRATIONS
        {
            return Err(ProtocolError::capacity());
        }
        let request_index = next
            .requests
            .iter()
            .position(|request| {
                request.request_id == request_id
                    && matches!(request.decision, StoredDecision::Pending)
            })
            .ok_or_else(ProtocolError::not_found)?;
        let registration = StoredRegistration {
            registration_id: format!("reg_{}", Uuid::new_v4().simple()),
            client: next.requests[request_index].client.clone(),
            approved_access: next.requests[request_index].requested_access.clone(),
            credential_hash: next.requests[request_index].credential_hash.clone(),
            created_at: Utc::now(),
            last_opened_at: None,
            revoked_at: None,
        };
        next.requests[request_index].decision = StoredDecision::Approved {
            registration_id: registration.registration_id.clone(),
        };
        next.registrations.push(registration);
        self.inner
            .state_file
            .write(&next)
            .map_err(|_| ProtocolError::internal())?;
        *state = next;
        Ok(())
    }

    async fn ensure_identity_scope(
        &self,
        approved_access: &pcp_rpc::RequestedAccess,
    ) -> std::result::Result<(), ProtocolError> {
        if !approved_access
            .scopes
            .iter()
            .any(|scope| scope == "user:self")
        {
            return Ok(());
        }
        let namespace = identity_scope_namespace(&self.inner.identity_id);
        let existing = self
            .inner
            .store
            .local_scope_names()
            .await
            .map_err(|_| ProtocolError::unavailable())?;
        if existing.iter().any(|scope| scope == &namespace) {
            return Ok(());
        }
        let principal = AccessPrincipal {
            principal_id: "service:pcp-enrollment".to_owned(),
            principal_type: AccessPrincipalType::Service,
            display_name: Some("PCP enrollment".to_owned()),
        };
        let operator = AccessSession::full_control(
            principal,
            format!("session:pcp-enrollment:{}", self.inner.service.generation),
            vec![namespace.clone()],
        );
        self.inner
            .store
            .create_scope(
                &operator,
                CreateScopeRequest {
                    namespace,
                    display_name: "User context".to_owned(),
                    description: Some(
                        "Identity-scoped durable context created for approved enrollment clients."
                            .to_owned(),
                    ),
                    parent_namespace: None,
                },
            )
            .await
            .map_err(|_| ProtocolError::unavailable())
    }

    async fn reject(&self, request_id: &str) -> std::result::Result<(), ProtocolError> {
        let mut state = self.inner.state.lock().await;
        let mut next = state.clone();
        cleanup_expired_requests(&mut next);
        let request = next
            .requests
            .iter_mut()
            .find(|request| {
                request.request_id == request_id
                    && matches!(request.decision, StoredDecision::Pending)
            })
            .ok_or_else(ProtocolError::not_found)?;
        request.decision = StoredDecision::Rejected;
        self.inner
            .state_file
            .write(&next)
            .map_err(|_| ProtocolError::internal())?;
        *state = next;
        Ok(())
    }

    async fn revoke(&self, registration_id: &str) -> std::result::Result<(), ProtocolError> {
        let mut state = self.inner.state.lock().await;
        let mut next = state.clone();
        let registration = next
            .registrations
            .iter_mut()
            .find(|registration| {
                registration.registration_id == registration_id && registration.revoked_at.is_none()
            })
            .ok_or_else(ProtocolError::not_found)?;
        registration.revoked_at = Some(Utc::now());
        for request in &mut next.requests {
            if matches!(
                &request.decision,
                StoredDecision::Approved { registration_id: approved } if approved == registration_id
            ) {
                request.decision = StoredDecision::Rejected;
            }
        }
        self.inner
            .state_file
            .write(&next)
            .map_err(|_| ProtocolError::internal())?;
        *state = next;
        drop(state);
        self.inner.sessions.lock().await.remove(registration_id);
        Ok(())
    }
}

fn result_for_request(
    request: &StoredRequest,
    registrations: &[StoredRegistration],
    session: Option<EnrollmentSession>,
) -> std::result::Result<EnrollmentResult, ProtocolError> {
    match &request.decision {
        StoredDecision::Pending => Ok(EnrollmentResult::Pending {
            request_id: request.request_id.clone(),
            requested_at: format_time(request.requested_at),
            expires_at: format_time(request.expires_at),
        }),
        StoredDecision::Rejected => Ok(EnrollmentResult::Rejected {
            request_id: request.request_id.clone(),
        }),
        StoredDecision::Approved { registration_id } => {
            if !registrations.iter().any(|registration| {
                registration.registration_id == *registration_id
                    && registration.revoked_at.is_none()
            }) {
                return Err(ProtocolError::not_found());
            }
            let session = session.ok_or_else(ProtocolError::unavailable)?;
            Ok(EnrollmentResult::Active { session })
        }
    }
}

fn access_session(
    registration: &StoredRegistration,
    identity_id: &str,
    generation: &str,
    read_scopes: Vec<String>,
) -> std::result::Result<AccessSession, ProtocolError> {
    let mode = match registration.approved_access.mode {
        RequestedAccessMode::Observe => AccessMode::Observe,
        RequestedAccessMode::Read => AccessMode::Read,
        RequestedAccessMode::Audit => AccessMode::Audit,
        RequestedAccessMode::Contribute => AccessMode::Contribute,
        RequestedAccessMode::Write => AccessMode::Write,
        RequestedAccessMode::Repair => AccessMode::Repair,
        RequestedAccessMode::Admin => AccessMode::Admin,
    };
    let scopes = registration
        .approved_access
        .scopes
        .iter()
        .map(|scope| {
            if scope == "user:self" {
                identity_scope_namespace(identity_id)
            } else {
                scope.clone()
            }
        })
        .collect();
    let principal = AccessPrincipal {
        principal_id: registration.client.principal.principal_id.clone(),
        principal_type: registration.client.principal.principal_type.clone(),
        display_name: registration.client.principal.display_name.clone(),
    };
    let session_id = format!("enrolled:{}:{generation}", registration.registration_id);
    let mut access = mode.session(
        principal.clone(),
        session_id.clone(),
        scopes,
        registration.approved_access.allow_cross_scope_derivation,
    );
    let primary_scopes = access
        .grants
        .iter()
        .map(|grant| grant.namespace.as_str())
        .collect::<HashSet<_>>();
    let read_scopes = read_scopes
        .into_iter()
        .filter(|scope| !primary_scopes.contains(scope.as_str()))
        .collect();
    access.grants.extend(
        AccessMode::Read
            .session(principal, session_id, read_scopes, false)
            .grants,
    );
    Ok(access)
}

fn identity_scope_namespace(identity_id: &str) -> String {
    format!("user:{identity_id}")
}

fn validate_begin(params: &BeginEnrollmentParams) -> std::result::Result<(), ProtocolError> {
    validate_client_access(&params.client, &params.requested_access)
        .map_err(|_| ProtocolError::invalid("client claim or requested access is invalid"))?;
    hash_credential(&params.credential)?;
    Ok(())
}

fn validate_client_access(
    client: &pcp_rpc::EnrollmentClientClaim,
    access: &pcp_rpc::RequestedAccess,
) -> Result<()> {
    let principal = &client.principal;
    anyhow::ensure!(
        !matches!(access.mode, RequestedAccessMode::Repair)
            || principal.principal_type != pcp_core::AccessPrincipalType::ModelClient,
        "model clients cannot request PCP repair access"
    );
    if principal.principal_id.trim().is_empty() || principal.principal_id.len() > 128 {
        anyhow::bail!("principal_id has invalid length");
    }
    if principal
        .display_name
        .as_ref()
        .is_some_and(|name| name.trim().is_empty() || name.len() > 128)
    {
        anyhow::bail!("display_name has invalid length");
    }
    if access.scopes.is_empty()
        || access.scopes.len() > 16
        || access
            .scopes
            .iter()
            .any(|scope| scope.trim().is_empty() || scope.len() > 128)
    {
        anyhow::bail!("requested scopes are invalid");
    }
    let unique_scopes = access.scopes.iter().collect::<HashSet<_>>();
    anyhow::ensure!(
        unique_scopes.len() == access.scopes.len(),
        "requested scopes contain duplicates"
    );
    Ok(())
}

fn validate_loaded_state(state: &EnrollmentState) -> Result<()> {
    let mut request_ids = HashSet::new();
    let mut registration_ids = HashSet::new();
    for registration in &state.registrations {
        anyhow::ensure!(
            registration_ids.insert(registration.registration_id.as_str()),
            "PCP enrollment state contains a duplicate registration_id"
        );
        anyhow::ensure!(
            registration.registration_id.starts_with("reg_")
                && valid_credential_hash(&registration.credential_hash),
            "PCP enrollment state contains an invalid registration"
        );
        validate_client_access(&registration.client, &registration.approved_access)?;
    }
    for request in &state.requests {
        anyhow::ensure!(
            request_ids.insert(request.request_id.as_str()),
            "PCP enrollment state contains a duplicate request_id"
        );
        anyhow::ensure!(
            request.request_id.starts_with("req_")
                && valid_credential_hash(&request.credential_hash),
            "PCP enrollment state contains an invalid request"
        );
        validate_client_access(&request.client, &request.requested_access)?;
        if let StoredDecision::Approved { registration_id } = &request.decision {
            let registration = state
                .registrations
                .iter()
                .find(|registration| {
                    registration.registration_id == *registration_id
                        && registration.revoked_at.is_none()
                })
                .context("approved enrollment request has no active registration")?;
            anyhow::ensure!(
                registration.client == request.client
                    && registration.approved_access == request.requested_access
                    && credential_matches(&registration.credential_hash, &request.credential_hash,),
                "approved enrollment request does not match its registration"
            );
        }
    }
    Ok(())
}

fn hash_credential(credential: &str) -> std::result::Result<String, ProtocolError> {
    if credential.len() != 64
        || !credential
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ProtocolError::invalid(
            "credential must be 32 random bytes encoded as lowercase hex",
        ));
    }
    Ok(format!("{:x}", Sha256::digest(credential.as_bytes())))
}

fn credential_matches(expected: &str, supplied: &str) -> bool {
    expected.len() == supplied.len()
        && expected
            .bytes()
            .zip(supplied.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn valid_credential_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn cleanup_expired_requests(state: &mut EnrollmentState) -> bool {
    let now = Utc::now();
    let before = state.requests.len();
    state.requests.retain(|request| {
        matches!(request.decision, StoredDecision::Approved { .. }) || request.expires_at > now
    });
    state.requests.len() != before
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).context("encode PCP enrollment response")
}

fn internal_public_error(_error: anyhow::Error) -> Vec<u8> {
    serde_json::to_vec(&EnrollmentError::public(
        "internal_error",
        "enrollment is temporarily unavailable",
    ))
    .unwrap_or_default()
}

fn internal_admin_error(_error: anyhow::Error) -> Vec<u8> {
    serde_json::to_vec(&EnrollmentError::admin(
        "internal_error",
        "enrollment administration is temporarily unavailable",
    ))
    .unwrap_or_default()
}

fn format_time(value: chrono::DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

struct ProtocolError {
    code: &'static str,
    message: &'static str,
}

impl ProtocolError {
    fn invalid(message: &'static str) -> Self {
        Self {
            code: "invalid_request",
            message,
        }
    }

    fn not_found() -> Self {
        Self {
            code: "not_found",
            message: "enrollment is not available",
        }
    }

    fn unavailable() -> Self {
        Self {
            code: "session_unavailable",
            message: "the approved PCP session is temporarily unavailable",
        }
    }

    fn internal() -> Self {
        Self {
            code: "internal_error",
            message: "enrollment is temporarily unavailable",
        }
    }

    fn capacity() -> Self {
        Self {
            code: "capacity_exceeded",
            message: "enrollment capacity is temporarily exhausted",
        }
    }
}

pub(crate) fn request_schema(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(bytes)
        .ok()?
        .get("schema")?
        .as_str()
        .map(str::to_owned)
}
