use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{EnrollmentConfig, ObserverConfig, ObserverService};
use pcp_client::{PcpApi, PcpTenantApi};
use pcp_core::{
    AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
    CreateScopeRequest, IngestPageRequest, PagePayload, WriteSummaryRequest,
};
use pcp_rpc::{
    BeginEnrollmentParams, EnrollmentAdminClient, EnrollmentAdminResult, EnrollmentClient,
    EnrollmentClientClaim, EnrollmentPrincipalClaim, EnrollmentResult, EnrollmentStatusParams,
    OpenEnrollmentSessionParams, PCP_ENROLLMENT_PROTOCOL_ID, RemotePcpClient, RequestedAccess,
    RequestedAccessMode,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

#[tokio::test]
async fn enrollment_approves_identity_bound_session_and_survives_generation_change() {
    let root = test_root("lifecycle");
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open enrollment test store"),
    );
    let identity_id = store.identity_id().to_owned();
    let identity_scope = format!("user:{identity_id}");
    let store: Arc<dyn PcpStore> = store;
    store
        .create_scope(
            &AccessSession::full_control(
                AccessPrincipal {
                    principal_id: "operator:enrollment-test".to_owned(),
                    principal_type: AccessPrincipalType::Service,
                    display_name: None,
                },
                "session:enrollment-test",
                vec![identity_scope.clone()],
            ),
            CreateScopeRequest {
                namespace: identity_scope.clone(),
                display_name: "Enrollment test identity".to_owned(),
                description: None,
                parent_namespace: None,
            },
        )
        .await
        .expect("create identity scope");
    let mut observer_config = ObserverConfig::for_test(root.clone(), identity_id.clone());
    observer_config.enrollment_enabled = true;
    let enrollment_config = EnrollmentConfig::for_test(root.clone());
    let admin_socket = enrollment_config.admin_socket_path.clone();
    let state_path = enrollment_config.state_path.clone();
    let mut observer = ObserverService::start(
        observer_config.clone(),
        enrollment_config,
        Arc::clone(&store),
    )
    .await
    .expect("start enrollment provider")
    .expect("provider enabled");
    let first_generation = observer.generation().to_owned();
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(observer_config.manifest_path()).expect("read discovery manifest"),
    )
    .expect("decode discovery manifest");
    assert!(manifest["offers"].as_array().unwrap().iter().any(|offer| {
        offer["protocol"] == PCP_ENROLLMENT_PROTOCOL_ID
            && offer["endpoint"]
                .as_str()
                .is_some_and(|value| value.starts_with("sockets/"))
    }));

    let credential = "ab".repeat(32);
    let public = EnrollmentClient::new(observer.socket_path());
    let pending = public
        .begin(begin_params(&credential))
        .await
        .expect("begin enrollment");
    let request_id = match pending.result {
        EnrollmentResult::Pending { request_id, .. } => request_id,
        other => panic!("expected pending enrollment, got {other:?}"),
    };
    let state_text = fs::read_to_string(&state_path).expect("read enrollment state");
    assert!(!state_text.contains(&credential));
    assert_eq!(
        fs::metadata(&state_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let admin = EnrollmentAdminClient::new(&admin_socket);
    let snapshot = admin.snapshot().await.expect("read pending enrollments");
    let snapshot_text = serde_json::to_string(&snapshot).expect("encode admin snapshot");
    assert!(!snapshot_text.contains(&credential));
    assert!(!snapshot_text.contains("credential_hash"));
    match snapshot.result {
        EnrollmentAdminResult::Snapshot {
            pending,
            registrations,
        } => {
            assert_eq!(pending.len(), 1);
            assert!(registrations.is_empty());
            assert_eq!(pending[0].request_id, request_id);
        }
        other => panic!("expected admin snapshot, got {other:?}"),
    }
    admin
        .approve(request_id.clone())
        .await
        .expect("approve enrollment");
    let active = public
        .status(EnrollmentStatusParams {
            request_id: request_id.clone(),
            credential: credential.clone(),
        })
        .await
        .expect("open approved enrollment");
    let first_session = match active.result {
        EnrollmentResult::Active { session } => session,
        other => panic!("expected active enrollment, got {other:?}"),
    };
    assert_eq!(first_session.service.generation, first_generation);
    assert!(std::path::Path::new(&first_session.endpoint).is_relative());
    assert_canonical_infra_socket_endpoint(&first_session.endpoint);
    assert!(
        first_session
            .access
            .allows(&format!("user:{identity_id}"), AccessPermission::ReadDetail)
    );
    assert!(
        first_session
            .access
            .allows(&format!("user:{identity_id}"), AccessPermission::Ingest)
    );
    assert!(
        !first_session
            .access
            .allows(&format!("user:{identity_id}"), AccessPermission::Write)
    );
    let remote =
        RemotePcpClient::connect_expected(root.join(&first_session.endpoint), "host:symbiont-d")
            .await
            .expect("connect identity-bound session");
    assert_eq!(remote.access(), &first_session.access);
    let ingested = remote
        .ingest_page(IngestPageRequest {
            namespace: identity_scope,
            kind: "conversation_event".to_owned(),
            observed_at: None,
            source_span: None,
            payload: Some(PagePayload {
                media_type: "text/plain".to_owned(),
                content: "A tenant can contribute a sealed source event.".to_owned(),
            }),
            source_refs: Vec::new(),
            facets: None,
            external_event_id: Some("enrollment:contribute:test".to_owned()),
        })
        .await
        .expect("ingest through contribute session");
    assert!(
        remote
            .write_summary(WriteSummaryRequest {
                target_page_id: ingested.page_id,
                target_revision_id: ingested.revision_id,
                expected_summary_revision_id: None,
                content: "A tenant must not publish maintained interpretation.".to_owned(),
                created_by: Actor {
                    actor_type: ActorType::Model,
                    actor_id: "model:tenant-test".to_owned(),
                },
                tool_or_model: None,
                provenance: Vec::new(),
                idempotency_key: Some("enrollment:contribute:summary".to_owned()),
            })
            .await
            .is_err()
    );
    let idempotent = public
        .begin(begin_params(&credential))
        .await
        .expect("repeat approved begin");
    match idempotent.result {
        EnrollmentResult::Active { session } => {
            assert_eq!(session.registration_id, first_session.registration_id);
            assert_eq!(session.endpoint, first_session.endpoint);
        }
        other => panic!("expected active idempotent begin, got {other:?}"),
    }

    let registration_id = first_session.registration_id.clone();
    observer.shutdown().await.expect("stop first provider");
    let enrollment_config = EnrollmentConfig::for_test(root.clone());
    let mut replacement =
        ObserverService::start(observer_config, enrollment_config, Arc::clone(&store))
            .await
            .expect("restart enrollment provider")
            .expect("replacement enabled");
    let replacement_public = EnrollmentClient::new(replacement.socket_path());
    let reopened = replacement_public
        .open_session(OpenEnrollmentSessionParams {
            registration_id: registration_id.clone(),
            credential: credential.clone(),
        })
        .await
        .expect("reopen persisted registration");
    let replacement_session = match reopened.result {
        EnrollmentResult::Active { session } => session,
        other => panic!("expected reopened enrollment, got {other:?}"),
    };
    assert_ne!(replacement_session.service.generation, first_generation);
    assert_eq!(replacement_session.registration_id, registration_id);
    let replacement_remote = RemotePcpClient::connect_expected(
        root.join(&replacement_session.endpoint),
        "host:symbiont-d",
    )
    .await
    .expect("connect replacement identity-bound session");

    let replacement_admin = EnrollmentAdminClient::new(&admin_socket);
    replacement_admin
        .revoke(registration_id.clone())
        .await
        .expect("revoke enrollment");
    assert!(replacement_remote.page_count(Vec::new()).await.is_err());
    assert!(
        replacement_public
            .open_session(OpenEnrollmentSessionParams {
                registration_id,
                credential,
            })
            .await
            .is_err()
    );
    replacement.shutdown().await.expect("stop replacement");
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn enrollment_requires_the_client_credential_for_status() {
    let root = test_root("credential");
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open credential test store"),
    );
    let identity_id = store.identity_id().to_owned();
    let store: Arc<dyn PcpStore> = store;
    let mut observer_config = ObserverConfig::for_test(root.clone(), identity_id);
    observer_config.enrollment_enabled = true;
    let mut observer = ObserverService::start(
        observer_config,
        EnrollmentConfig::for_test(root.clone()),
        store,
    )
    .await
    .expect("start enrollment provider")
    .expect("provider enabled");
    let credential = "cd".repeat(32);
    let public = EnrollmentClient::new(observer.socket_path());
    let request_id = match public
        .begin(begin_params(&credential))
        .await
        .expect("begin enrollment")
        .result
    {
        EnrollmentResult::Pending { request_id, .. } => request_id,
        other => panic!("expected pending enrollment, got {other:?}"),
    };
    assert!(
        public
            .status(EnrollmentStatusParams {
                request_id,
                credential: "ef".repeat(32),
            })
            .await
            .is_err()
    );
    let mut duplicate_scope = begin_params(&"12".repeat(32));
    duplicate_scope.requested_access.scopes = vec!["user:self".to_owned(); 2];
    assert!(public.begin(duplicate_scope).await.is_err());
    observer.shutdown().await.expect("stop provider");
    let _ = tokio::fs::remove_dir_all(root).await;
}

fn assert_canonical_infra_socket_endpoint(endpoint: &str) {
    let opaque = endpoint
        .strip_prefix("sockets/")
        .and_then(|value| value.strip_suffix(".sock"))
        .expect("canonical Infra Unix socket endpoint");
    assert!(!opaque.is_empty() && opaque.len() <= 16);
    assert!(opaque.bytes().next().unwrap().is_ascii_alphanumeric());
    assert!(
        opaque
            .bytes()
            .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') })
    );
}

fn begin_params(credential: &str) -> BeginEnrollmentParams {
    BeginEnrollmentParams {
        client: EnrollmentClientClaim {
            principal: EnrollmentPrincipalClaim {
                principal_id: "host:symbiont-d".to_owned(),
                principal_type: AccessPrincipalType::Host,
                display_name: Some("Symbiont".to_owned()),
            },
        },
        requested_access: RequestedAccess {
            mode: RequestedAccessMode::Contribute,
            scopes: vec![
                "user:self".to_owned(),
                "project:symbiont-d".to_owned(),
                "conversation:symbiont-d".to_owned(),
            ],
            allow_cross_scope_derivation: false,
        },
        credential: credential.to_owned(),
    }
}

fn test_root(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::path::PathBuf::from("/tmp").join(format!("pcpe-{label}-{}", nonce % 1_000_000_000));
    fs::create_dir_all(&root).expect("create enrollment test root");
    root
}
