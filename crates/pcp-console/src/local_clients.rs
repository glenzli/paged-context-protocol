//! Console-owned local setup records. Runtime remains the sole grant authority;
//! MCP owns credential serialization and authenticated enrollment sessions.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::{Path as RoutePath, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use pcp_client::PcpTenantApi;
use pcp_core::AccessPrincipalType;
use pcp_mcp::enrollment::{self, LocalEnrollmentInfo, LocalEnrollmentProgress};
use pcp_rpc::{
    EnrollmentAdminResult, PendingEnrollmentView, RegisteredClientView, RequestedAccess,
    RequestedAccessMode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::AppState;

const RECORD_VERSION: u32 = 1;
const MAX_PROFILES: usize = 128;
const MAX_RECORD_BYTES: u64 = 16 * 1024;

#[derive(Default)]
pub(super) struct LocalClients {
    operation: Mutex<()>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LocalMode {
    #[default]
    Read,
    Contribute,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateInput {
    operation_id: String,
    display_name: String,
    #[serde(default)]
    mode: LocalMode,
    scopes: Vec<String>,
}

impl CreateInput {
    fn normalized(mut self) -> LocalResult<Self> {
        validate_id(&self.operation_id)?;
        self.display_name = self.display_name.trim().to_owned();
        if self.display_name.is_empty()
            || self.display_name.len() > 120
            || self.display_name.chars().any(char::is_control)
        {
            return Err(LocalError::validation(
                "Use a readable client name of at most 120 UTF-8 bytes.",
            ));
        }
        if self.scopes.is_empty() || self.scopes.len() > 16 {
            return Err(LocalError::validation(
                "Choose between one and 16 explicit Scopes.",
            ));
        }
        if self.scopes.iter().any(|scope| {
            scope.trim() != scope
                || scope.is_empty()
                || scope.len() > 128
                || scope.chars().any(char::is_control)
        }) {
            return Err(LocalError::validation("The selected Scopes are invalid."));
        }
        self.scopes.sort();
        if self.scopes.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(LocalError::validation("Choose each Scope only once."));
        }
        Ok(self)
    }

    fn access(&self) -> RequestedAccess {
        RequestedAccess {
            mode: match self.mode {
                LocalMode::Read => RequestedAccessMode::Read,
                LocalMode::Contribute => RequestedAccessMode::Contribute,
            },
            scopes: self.scopes.clone(),
            read_all_scopes: false,
            allow_cross_scope_derivation: false,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileRecord {
    version: u32,
    input: CreateInput,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalClientView {
    id: String,
    display_name: String,
    principal_id: String,
    mode: LocalMode,
    scopes: Vec<String>,
    status: &'static str,
    can_resume_registration: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    registration_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_opened_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

impl LocalClientView {
    fn unavailable(input: &CreateInput, reason: &'static str) -> Self {
        Self {
            id: input.operation_id.clone(),
            display_name: input.display_name.clone(),
            principal_id: String::new(),
            mode: input.mode.clone(),
            scopes: input.scopes.clone(),
            status: "unavailable",
            can_resume_registration: false,
            request_id: None,
            registration_id: None,
            last_opened_at: None,
            reason: Some(reason),
        }
    }
}

#[derive(Debug)]
pub(super) struct LocalError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    not_applied: bool,
}

type LocalResult<T> = Result<T, LocalError>;

impl LocalError {
    fn validation(message: &'static str) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "local_client_validation_failed",
            message,
            not_applied: true,
        }
    }

    fn unavailable(message: &'static str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "local_client_unavailable",
            message,
            not_applied: false,
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "local_client_not_found",
            message: "No local client was prepared with this operation ID.",
            not_applied: false,
        }
    }

    fn conflict(message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "local_client_operation_conflict",
            message,
            not_applied: true,
        }
    }
}

impl IntoResponse for LocalError {
    fn into_response(self) -> Response {
        // Never expose an underlying enrollment/JSON error or credential bytes.
        let mut value = json!({"error":self.message,"code":self.code});
        if self.not_applied {
            value["mutationOutcome"] = json!("not_applied");
        }
        (self.status, [("cache-control", "no-store")], Json(value)).into_response()
    }
}

struct Environment {
    root: PathBuf,
    manifest: PathBuf,
    mcp_binary: PathBuf,
}

impl Environment {
    async fn read(state: &AppState) -> LocalResult<Self> {
        let runtime = state.runtime.as_ref().ok_or_else(|| {
            LocalError::unavailable("Local client setup requires a managed PCP Console.")
        })?;
        let home = fs::canonicalize(runtime.home())
            .map_err(|_| LocalError::unavailable("The managed PCP home is unavailable."))?;
        private_directory(&home)?;
        let root = home.join("local-clients");
        if fs::symlink_metadata(&root).is_ok() {
            private_directory(&root)?;
        }
        let mcp_binary = executable(runtime.mcp_binary_candidates())?;
        let manifest = runtime.discovery_manifest().await.ok_or_else(|| {
            LocalError::unavailable(
                "Runtime Discovery is disabled; local client setup is unavailable.",
            )
        })?;
        enrollment::validate_local_discovery(&manifest, state.client.identity_id()).map_err(
            |_| {
                LocalError::unavailable(
                    "A compatible Runtime Discovery enrollment endpoint is unavailable.",
                )
            },
        )?;
        Ok(Self {
            root,
            manifest,
            mcp_binary,
        })
    }

    fn profile(&self, id: &str) -> LocalResult<PathBuf> {
        validate_id(id)?;
        let profile = self.root.join(id);
        match fs::symlink_metadata(&profile) {
            Ok(_) => private_directory(&profile)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(LocalError::not_found());
            }
            Err(_) => {
                return Err(LocalError::unavailable(
                    "The local client directory is unreadable.",
                ));
            }
        }
        Ok(profile)
    }

    fn record(&self, id: &str) -> LocalResult<(PathBuf, ProfileRecord)> {
        let profile = self.profile(id)?;
        let record: ProfileRecord = read_record(&profile.join("profile.json"))?;
        if record.version != RECORD_VERSION
            || record.input.operation_id != id
            || record.input.clone().normalized().as_ref().ok() != Some(&record.input)
        {
            return Err(LocalError::unavailable(
                "The local client setup record is invalid.",
            ));
        }
        Ok((profile, record))
    }
}

struct Snapshot {
    pending: Vec<PendingEnrollmentView>,
    registrations: Vec<RegisteredClientView>,
}

async fn snapshot(state: &AppState) -> LocalResult<Snapshot> {
    let response = state.enrollment.snapshot().await.map_err(|_| {
        LocalError::unavailable(
            "Runtime enrollment status is unavailable. Refresh to recover the current result.",
        )
    })?;
    match response.result {
        EnrollmentAdminResult::Snapshot {
            pending,
            registrations,
        } => Ok(Snapshot {
            pending,
            registrations,
        }),
        _ => Err(LocalError::unavailable(
            "Runtime returned an unexpected enrollment status.",
        )),
    }
}

pub(super) async fn list(State(state): State<AppState>) -> LocalResult<Json<Value>> {
    let _operation = state.local_clients.operation.lock().await;
    let environment = match Environment::read(&state).await {
        Ok(environment) => environment,
        Err(error) => {
            return Ok(Json(
                json!({"available":false,"reason":error.message,"clients":[],"scopeOptions":[]}),
            ));
        }
    };
    let snapshot = match snapshot(&state).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Ok(Json(
                json!({"available":false,"reason":error.message,"clients":[],"scopeOptions":[]}),
            ));
        }
    };
    let scopes = scope_options(&state).await?;
    let mut clients = Vec::new();
    if environment.root.exists() {
        let entries = fs::read_dir(&environment.root)
            .map_err(|_| LocalError::unavailable("The local client list is unreadable."))?;
        let mut ids = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|id| valid_id(id))
            .collect::<Vec<_>>();
        ids.sort();
        if ids.len() > MAX_PROFILES {
            return Err(LocalError::unavailable(
                "The local client list exceeds its supported limit.",
            ));
        }
        for id in ids {
            let (profile, record) = environment.record(&id)?;
            clients.push(reconcile(&state, &profile, &record.input, &snapshot).await);
        }
    }
    Ok(Json(
        json!({"available":true,"clients":clients,"scopeOptions":scopes}),
    ))
}

pub(super) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    input: Result<Json<CreateInput>, JsonRejection>,
) -> LocalResult<Json<LocalClientView>> {
    require_mutation(&headers)?;
    let Json(input) = input.map_err(|_| LocalError::validation("The local client request is invalid. Use only the supported name, mode, Scopes and operation ID."))?;
    let input = input.normalized()?;
    let _operation = state.local_clients.operation.lock().await;
    let environment = Environment::read(&state).await?;
    let scopes = scope_options(&state).await?;
    let allowed = scopes
        .iter()
        .filter_map(|scope| scope["namespace"].as_str())
        .collect::<BTreeSet<_>>();
    if input
        .scopes
        .iter()
        .any(|scope| !allowed.contains(scope.as_str()))
    {
        return Err(LocalError::validation(
            "A selected Scope is no longer available. Refresh the Scope list.",
        ));
    }
    ensure_directory(&environment.root)?;
    let _file_lock = lock_store(&environment.root)?;
    let profile = environment.root.join(&input.operation_id);
    match fs::symlink_metadata(&profile) {
        Ok(_) => {
            private_directory(&profile)?;
            prepare_record(&profile, &input)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let count = fs::read_dir(&environment.root)
                .map_err(|_| LocalError::unavailable("The local client list is unreadable."))?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_name().to_str().is_some_and(valid_id))
                .count();
            if count >= MAX_PROFILES {
                return Err(LocalError::conflict(
                    "The local client profile limit has been reached.",
                ));
            }
            ensure_directory(&profile)?;
            prepare_record(&profile, &input)?;
        }
        Err(_) => {
            return Err(LocalError::unavailable(
                "The local client directory is unreadable.",
            ));
        }
    }
    let state_path = profile.join("enrollment.json");
    match fs::symlink_metadata(&state_path) {
        Ok(_) => {
            enrollment_info(&state, &profile, &input)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            enrollment::prepare_local_enrollment(&state_path, input.display_name.clone(), input.access(), environment.manifest, state.client.identity_id())
                .map_err(|_| LocalError::unavailable("The private local enrollment could not be prepared. Refresh this operation to recover its state."))?;
        }
        Err(_) => {
            return Err(LocalError::unavailable(
                "The private local enrollment is unreadable.",
            ));
        }
    }
    // Never re-submit a known registration or terminal request on a retry.
    let current = snapshot(&state).await?;
    let recovered = reconcile(&state, &profile, &input, &current).await;
    if recovered.status != "unavailable" {
        return Ok(Json(recovered));
    }
    let info = enrollment_info(&state, &profile, &input)?;
    if info.request_id.is_none() && info.registration_id.is_none() {
        enrollment::submit_local_enrollment(&state_path).await.map_err(|_| LocalError::unavailable("The enrollment submission result is unknown. Refresh this operation before taking another action."))?;
    }
    Ok(Json(
        reconcile(&state, &profile, &input, &snapshot(&state).await?).await,
    ))
}

pub(super) async fn inspect(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> LocalResult<Json<LocalClientView>> {
    let _operation = state.local_clients.operation.lock().await;
    let environment = Environment::read(&state).await?;
    let (profile, record) = environment.record(&id)?;
    Ok(Json(
        reconcile(&state, &profile, &record.input, &snapshot(&state).await?).await,
    ))
}

pub(super) async fn approve(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> LocalResult<Json<LocalClientView>> {
    require_mutation(&headers)?;
    let _operation = state.local_clients.operation.lock().await;
    let environment = Environment::read(&state).await?;
    let (profile, record) = environment.record(&id)?;
    let _file_lock = lock_store(&environment.root)?;
    let current = snapshot(&state).await?;
    let view = reconcile(&state, &profile, &record.input, &current).await;
    if view.status == "ready" {
        return Ok(Json(view));
    }
    if view.status != "pending" {
        return Err(LocalError::conflict(
            "This client has no current pending grant to approve. Refresh its state.",
        ));
    }
    // Bind the immutable browser review to the current Runtime request. Runtime
    // still performs the real approval and all access-policy validation.
    let info = enrollment_info(&state, &profile, &record.input)?;
    let request = current
        .pending
        .iter()
        .find(|pending| matches_pending(&info, &record.input, pending))
        .ok_or_else(|| {
            LocalError::conflict("The pending grant no longer matches this local setup.")
        })?;
    state.enrollment.approve(&request.request_id).await.map_err(|_| LocalError::unavailable("The approval result is unknown. Refresh this operation to recover the current grant."))?;
    Ok(Json(
        reconcile(&state, &profile, &record.input, &snapshot(&state).await?).await,
    ))
}

pub(super) async fn revoke(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> LocalResult<Json<LocalClientView>> {
    require_mutation(&headers)?;
    let _operation = state.local_clients.operation.lock().await;
    let environment = Environment::read(&state).await?;
    let (profile, record) = environment.record(&id)?;
    let _file_lock = lock_store(&environment.root)?;
    let view = reconcile(&state, &profile, &record.input, &snapshot(&state).await?).await;
    if view.status == "revoked" {
        return Ok(Json(view));
    }
    if view.status != "ready" {
        return Err(LocalError::conflict(
            "Only a current approved local client can be revoked here.",
        ));
    }
    let registration_id = view
        .registration_id
        .ok_or_else(|| LocalError::unavailable("The local registration is incomplete."))?;
    state.enrollment.revoke(registration_id).await.map_err(|_| LocalError::unavailable("The revocation result is unknown. Refresh the client before taking another action."))?;
    Ok(Json(
        reconcile(&state, &profile, &record.input, &snapshot(&state).await?).await,
    ))
}

pub(super) async fn config(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> LocalResult<Json<Value>> {
    let _operation = state.local_clients.operation.lock().await;
    let environment = Environment::read(&state).await?;
    let (profile, record) = environment.record(&id)?;
    let client = reconcile(&state, &profile, &record.input, &snapshot(&state).await?).await;
    if client.status != "ready" {
        return Err(LocalError::conflict(
            "MCP configuration is available only for a current approved local client.",
        ));
    }
    let config = render_config(
        &environment.mcp_binary,
        &profile.join("enrollment.json"),
        &client,
    );
    Ok(Json(json!({"client":client,"config":config})))
}

fn render_config(binary: &Path, enrollment_file: &Path, client: &LocalClientView) -> Value {
    json!({"mcpServers":{format!("pcp-{}", client.id):{
        "command":binary,
        "args":[],
        "env":{
            "PCP_CLIENT_ID":client.principal_id,
            "PCP_ENROLLMENT_FILE":enrollment_file,
            "PCP_MCP_SURFACE":"generic",
            "PCP_MCP_TOOLSET":"context"
        }
    }}})
}

fn enrollment_info(
    state: &AppState,
    profile: &Path,
    input: &CreateInput,
) -> LocalResult<LocalEnrollmentInfo> {
    let info = enrollment::inspect_local_enrollment(&profile.join("enrollment.json")).map_err(|_| LocalError::unavailable("The private enrollment file is missing or invalid. Its credential is never returned to the browser."))?;
    if info.identity_id != state.client.identity_id()
        || !info.principal_id.starts_with("client:local:")
        || info.principal_name.as_deref() != Some(input.display_name.as_str())
        || info.requested_access != input.access()
    {
        return Err(LocalError::unavailable(
            "The private enrollment does not match this local client or Runtime identity.",
        ));
    }
    Ok(info)
}

fn matches_principal(info: &LocalEnrollmentInfo, claim: &pcp_rpc::EnrollmentClientClaim) -> bool {
    claim.principal.principal_id == info.principal_id
        && claim.principal.principal_type == AccessPrincipalType::ModelClient
        && claim.principal.display_name == info.principal_name
}

fn matches_pending(
    info: &LocalEnrollmentInfo,
    input: &CreateInput,
    pending: &PendingEnrollmentView,
) -> bool {
    matches_principal(info, &pending.client)
        && pending.requested_access == input.access()
        && info
            .request_id
            .as_ref()
            .is_none_or(|id| id == &pending.request_id)
}

fn matches_registration(
    info: &LocalEnrollmentInfo,
    input: &CreateInput,
    registered: &RegisteredClientView,
) -> bool {
    matches_principal(info, &registered.client)
        && registered.approved_access == input.access()
        && info
            .registration_id
            .as_ref()
            .is_none_or(|id| id == &registered.registration_id)
}

async fn reconcile(
    state: &AppState,
    profile: &Path,
    input: &CreateInput,
    snapshot: &Snapshot,
) -> LocalClientView {
    let mut view = LocalClientView::unavailable(
        input,
        "Local setup is incomplete. Retry the original operation explicitly to finish preparing it.",
    );
    if fs::symlink_metadata(profile.join("enrollment.json"))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        // No Runtime begin can precede publishing this credential. A durable
        // reviewed record and a current snapshot make explicit resume safe.
        view.can_resume_registration = true;
        return view;
    }
    let info = match enrollment_info(state, profile, input) {
        Ok(info) => info,
        Err(error) => {
            view.reason = Some(error.message);
            return view;
        }
    };
    view.principal_id = info.principal_id.clone();
    view.request_id = info.request_id.clone();
    view.registration_id = info.registration_id.clone();
    if let Some(registered) = snapshot
        .registrations
        .iter()
        .find(|registered| matches_registration(&info, input, registered))
    {
        // Authenticate with the private credential and validate the RPC identity,
        // not just the operator's enrollment list, before reporting readiness.
        if enrollment::activate_local_enrollment(
            &profile.join("enrollment.json"),
            &registered.registration_id,
        )
        .await
        .is_err()
        {
            view.reason = Some(
                "The approved local credential could not open its own Runtime session. Refresh to recover.",
            );
            return view;
        }
        view.status = "ready";
        view.registration_id = Some(registered.registration_id.clone());
        view.last_opened_at = registered.last_opened_at.clone();
        view.reason = None;
    } else if info.registration_id.is_some() {
        if snapshot.registrations.iter().any(|registered| {
            registered.registration_id == info.registration_id.as_deref().unwrap_or_default()
                || registered.client.principal.principal_id == info.principal_id
        }) {
            view.reason = Some(
                "The current Runtime grant no longer matches the reviewed local client request.",
            );
        } else {
            view.status = "revoked";
            view.reason = Some(
                "This registration is no longer active in Runtime. Create a new local setup to request access again.",
            );
        }
    } else if let Some(pending) = snapshot
        .pending
        .iter()
        .find(|pending| matches_pending(&info, input, pending))
    {
        view.status = "pending";
        view.request_id = Some(pending.request_id.clone());
        view.reason = None;
    } else if info.request_id.is_some() {
        match enrollment::local_enrollment_decision(&profile.join("enrollment.json")).await {
            Ok(LocalEnrollmentProgress::Rejected { .. }) => {
                view.status = "rejected";
                view.reason = Some(
                    "This enrollment was rejected. Create a new local setup to request access again.",
                );
            }
            _ => {
                view.reason = Some(
                    "No matching pending grant is available. It may have expired or changed. Refresh before creating a new request.",
                )
            }
        }
    } else if !snapshot
        .pending
        .iter()
        .any(|pending| pending.client.principal.principal_id == info.principal_id)
        && !snapshot
            .registrations
            .iter()
            .any(|registered| registered.client.principal.principal_id == info.principal_id)
    {
        view.can_resume_registration = true;
    }
    view
}

async fn scope_options(state: &AppState) -> LocalResult<Vec<Value>> {
    let scopes = crate::all_scopes(&state.client)
        .await
        .map_err(|_| LocalError::unavailable("The current Scope list is unavailable."))?;
    Ok(scope_option_values(scopes, state.client.identity_id()))
}

fn scope_option_values(scopes: Vec<pcp_core::Scope>, identity_id: &str) -> Vec<Value> {
    let personal_scope = format!("user:{identity_id}");
    let mut options = scopes
        .into_iter()
        .map(|scope| {
            let namespace = if scope.namespace == personal_scope {
                "user:self".to_owned()
            } else {
                scope.namespace
            };
            json!({"namespace":namespace,"displayName":scope.display_name})
        })
        .collect::<Vec<_>>();
    // Runtime resolves this explicit identity-bound alias after approval; no
    // Scope or project is created merely by opening this setup interface.
    if !options
        .iter()
        .any(|option| option["namespace"] == "user:self")
    {
        options.push(
            json!({"namespace":"user:self","displayName":"User context (this PCP identity)"}),
        );
    }
    options.sort_by(|a, b| a["namespace"].as_str().cmp(&b["namespace"].as_str()));
    options.dedup_by(|a, b| a["namespace"] == b["namespace"]);
    options
}

fn require_mutation(headers: &HeaderMap) -> LocalResult<()> {
    if headers
        .get("x-pcp-console")
        .and_then(|value| value.to_str().ok())
        != Some("1")
    {
        return Err(LocalError {
            status: StatusCode::FORBIDDEN,
            code: "local_client_mutation_header_required",
            message: "The PCP Console mutation header is required.",
            not_applied: true,
        });
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn validate_id(id: &str) -> LocalResult<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(LocalError::validation(
            "Use a canonical lowercase UUID as the operation ID.",
        ))
    }
}

fn private_directory(path: &Path) -> LocalResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| LocalError::unavailable("A private local setup directory is unavailable."))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o777 != 0o700
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(LocalError::unavailable(
            "Local client directories must be real, owned directories with mode 0700.",
        ));
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> LocalResult<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                File::open(parent)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| {
                        LocalError::unavailable("The local setup directory could not be persisted.")
                    })?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => {
            return Err(LocalError::unavailable(
                "The private local setup directory could not be created.",
            ));
        }
    }
    private_directory(path)
}

fn private_file(path: &Path) -> LocalResult<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| {
            LocalError::unavailable("The private local setup record is unavailable or incomplete.")
        })?;
    let metadata = file
        .metadata()
        .map_err(|_| LocalError::unavailable("The private local setup record is unreadable."))?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.len() > MAX_RECORD_BYTES
    {
        return Err(LocalError::unavailable(
            "Local client records must be owned regular files with mode 0600 and bounded size.",
        ));
    }
    Ok(file)
}

fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> LocalResult<T> {
    let mut bytes = Vec::new();
    private_file(path)?
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            LocalError::unavailable("The private local setup record could not be read.")
        })?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(LocalError::unavailable(
            "The local setup record is too large.",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| LocalError::unavailable("The private local setup record is invalid."))
}

fn prepare_record(profile: &Path, input: &CreateInput) -> LocalResult<()> {
    let path = profile.join("profile.json");
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            let record: ProfileRecord = read_record(&path)?;
            if record.version != RECORD_VERSION || &record.input != input {
                return Err(LocalError::conflict(
                    "This operation ID already belongs to a different local client request.",
                ));
            }
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(LocalError::unavailable(
                "The local setup record is unreadable.",
            ));
        }
    }
    // The immutable record is always published before the credential and before
    // Runtime begin. Only an explicit retry may complete this pre-submission
    // crash point; an existing credential without its record is never adopted.
    match fs::symlink_metadata(profile.join("enrollment.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => {
            return Err(LocalError::unavailable(
                "The local enrollment has no matching setup record. Preserve its private files and inspect the registration.",
            ));
        }
    }
    let temporary = path.with_extension("json.new");
    match fs::symlink_metadata(&temporary) {
        Ok(_) => {
            private_file(&temporary)?;
            fs::remove_file(&temporary).map_err(|_| {
                LocalError::unavailable("The interrupted local setup could not be recovered.")
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(LocalError::unavailable(
                "The interrupted local setup is unreadable.",
            ));
        }
    }
    write_record(
        &path,
        &ProfileRecord {
            version: RECORD_VERSION,
            input: input.clone(),
        },
    )
}

fn write_record(path: &Path, record: &ProfileRecord) -> LocalResult<()> {
    let mut bytes = serde_json::to_vec_pretty(record)
        .map_err(|_| LocalError::unavailable("The local setup record could not be encoded."))?;
    bytes.push(b'\n');
    let temporary = path.with_extension("json.new");
    // After a crash before the rename, no enrollment could have been submitted.
    // Keep that partial record visible for recovery rather than overwrite it.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)
        .map_err(|_| LocalError::unavailable("The local setup record could not be prepared."))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| LocalError::unavailable("The local setup record could not be persisted."))?;
    fs::rename(&temporary, path)
        .map_err(|_| LocalError::unavailable("The local setup record could not be published."))?;
    File::open(path.parent().expect("fixed profile parent"))
        .and_then(|file| file.sync_all())
        .map_err(|_| {
            LocalError::unavailable("The local setup record directory could not be persisted.")
        })?;
    Ok(())
}

fn lock_store(root: &Path) -> LocalResult<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(".lock"))
        .map_err(|_| LocalError::unavailable("The local setup lock is unavailable."))?;
    let metadata = lock
        .metadata()
        .map_err(|_| LocalError::unavailable("The local setup lock is unreadable."))?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(LocalError::unavailable(
            "The local setup lock is not a private owned file.",
        ));
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(LocalError::conflict(
            "Another local setup operation is in progress. Refresh before trying again.",
        ));
    }
    Ok(lock)
}

fn executable(candidates: Vec<PathBuf>) -> LocalResult<PathBuf> {
    for candidate in candidates {
        if !candidate.is_absolute() {
            continue;
        }
        match fs::canonicalize(candidate) {
            Ok(path) => {
                let metadata = fs::metadata(&path).map_err(|_| {
                    LocalError::unavailable("The installed PCP MCP executable is unreadable.")
                })?;
                if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                    return Ok(path);
                }
                return Err(LocalError::unavailable(
                    "The installed pcp-mcp is not an executable regular file.",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(LocalError::unavailable(
                    "The installed PCP MCP executable cannot be resolved.",
                ));
            }
        }
    }
    Err(LocalError::unavailable(
        "The local pcp-mcp executable is missing. Install the PCP MCP binary beside Console or in the managed home/bin directory.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn input() -> CreateInput {
        CreateInput {
            operation_id: "cf990fd7-50d3-42a0-837f-9ce1972ab06c".to_owned(),
            display_name: "Other agent".to_owned(),
            mode: LocalMode::Read,
            scopes: vec!["user:self".to_owned()],
        }
    }

    fn root(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "pcp-local-clients-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        fs::canonicalize(directory).unwrap()
    }

    #[test]
    fn local_grants_are_explicit_bounded_and_read_only_by_default() {
        let parsed: CreateInput = serde_json::from_value(json!({"operationId":input().operation_id,"displayName":"  Other agent  ","scopes":["user:self"]})).unwrap();
        let normalized = parsed.normalized().unwrap();
        assert_eq!(normalized, input());
        assert_eq!(normalized.access().mode, RequestedAccessMode::Read);
        assert!(!normalized.access().read_all_scopes);
        assert!(!normalized.access().allow_cross_scope_derivation);
        for mode in ["admin", "write", "repair", "audit", "observe"] {
            assert!(serde_json::from_value::<CreateInput>(json!({"operationId":input().operation_id,"displayName":"agent","mode":mode,"scopes":["user:self"]})).is_err());
        }
        for scopes in [
            vec![],
            vec!["user:self".to_owned(); 2],
            vec!["user:self".to_owned(); 17],
        ] {
            assert!(CreateInput { scopes, ..input() }.normalized().is_err());
        }
        assert!(serde_json::from_value::<CreateInput>(json!({"operationId":input().operation_id,"displayName":"agent","scopes":["user:self"],"command":"/tmp/arbitrary"})).is_err());
    }

    #[test]
    fn operation_ids_cannot_escape_the_managed_directory() {
        assert!(valid_id(&input().operation_id));
        for id in [
            "../escape",
            "/tmp/client",
            ".",
            "CF990FD7-50D3-42A0-837F-9CE1972AB06C",
            "cf990fd7/50d3-42a0-837f-9ce1972ab06c",
        ] {
            assert!(!valid_id(id));
        }
    }

    #[test]
    fn scope_options_collapse_only_the_current_identity_alias() {
        let scope = |namespace: &str, display_name: &str| pcp_core::Scope {
            namespace: namespace.to_owned(),
            display_name: display_name.to_owned(),
            description: None,
            parent_namespace: None,
            created_at: String::new(),
            updated_at: String::new(),
            page_count: 0,
        };
        let options = scope_option_values(
            vec![
                scope("user:current-identity", "Personal memory"),
                scope("user:another-identity", "Another user's memory"),
                scope("user:current-identity:history", "Historical memory"),
                scope("project:example", "Example project"),
            ],
            "current-identity",
        );
        assert_eq!(options.len(), 4);
        assert_eq!(
            options
                .iter()
                .filter(|option| option["namespace"] == "user:self")
                .count(),
            1
        );
        assert!(
            !options
                .iter()
                .any(|option| option["namespace"] == "user:current-identity")
        );
        assert!(
            options
                .iter()
                .any(|option| option["namespace"] == "user:self"
                    && option["displayName"] == "Personal memory")
        );
        for preserved in [
            "user:another-identity",
            "user:current-identity:history",
            "project:example",
        ] {
            assert!(
                options
                    .iter()
                    .any(|option| option["namespace"] == preserved)
            );
        }
        let before_personal_scope_exists = scope_option_values(Vec::new(), "current-identity");
        assert_eq!(before_personal_scope_exists.len(), 1);
        assert_eq!(before_personal_scope_exists[0]["namespace"], "user:self");
        let both_aliases = scope_option_values(
            vec![
                scope("user:current-identity", "Personal memory"),
                scope("user:self", "Personal alias"),
            ],
            "current-identity",
        );
        assert_eq!(both_aliases.len(), 1);
        assert_eq!(both_aliases[0]["namespace"], "user:self");
    }

    #[test]
    fn client_names_use_the_runtime_utf8_byte_bound() {
        let name = "中".repeat(40);
        assert_eq!(name.len(), 120);
        assert!(
            CreateInput {
                display_name: name,
                ..input()
            }
            .normalized()
            .is_ok()
        );
        assert!(
            CreateInput {
                display_name: "中".repeat(41),
                ..input()
            }
            .normalized()
            .is_err()
        );
        assert!(
            CreateInput {
                display_name: "client\nname".to_owned(),
                ..input()
            }
            .normalized()
            .is_err()
        );
    }

    #[test]
    fn records_and_locks_reject_symlinks_and_unsafe_modes() {
        let root = root("paths");
        let target = root.join("target");
        ensure_directory(&target).unwrap();
        symlink(&target, root.join("linked")).unwrap();
        assert!(ensure_directory(&root.join("linked")).is_err());
        let record = target.join("profile.json");
        write_record(
            &record,
            &ProfileRecord {
                version: RECORD_VERSION,
                input: input(),
            },
        )
        .unwrap();
        let decoded: ProfileRecord = read_record(&record).unwrap();
        assert_eq!(decoded.input, input());
        assert_eq!(
            fs::metadata(&record).unwrap().permissions().mode() & 0o777,
            0o600
        );
        symlink(&record, root.join("record-link")).unwrap();
        assert!(read_record::<ProfileRecord>(&root.join("record-link")).is_err());
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_record::<ProfileRecord>(&record).is_err());
        let lock = lock_store(&root).unwrap();
        assert!(lock_store(&root).is_err());
        drop(lock);
        assert!(lock_store(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn setup_retries_bind_the_original_request_and_recover_before_submission() {
        let root = root("recovery");
        prepare_record(&root, &input()).unwrap();
        let before = fs::read(root.join("profile.json")).unwrap();
        prepare_record(&root, &input()).unwrap();
        assert_eq!(fs::read(root.join("profile.json")).unwrap(), before);
        assert!(
            prepare_record(
                &root,
                &CreateInput {
                    mode: LocalMode::Contribute,
                    ..input()
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(root.join("profile.json")).unwrap(), before);

        fs::remove_file(root.join("profile.json")).unwrap();
        let temporary = root.join("profile.json.new");
        fs::write(&temporary, b"{unfinished").unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
        prepare_record(&root, &input()).unwrap();
        assert!(!temporary.exists());
        assert_eq!(fs::read(root.join("profile.json")).unwrap(), before);

        fs::remove_file(root.join("profile.json")).unwrap();
        fs::write(root.join("enrollment.json"), b"private-orphaned-state").unwrap();
        assert!(prepare_record(&root, &input()).is_err());
        assert!(!root.join("profile.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn errors_expose_only_safe_messages_and_bound_rejections() {
        let validation = LocalError::validation("Choose an explicit Scope.").into_response();
        assert_eq!(validation.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = axum::body::to_bytes(validation.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["mutationOutcome"], "not_applied");
        assert_eq!(body["code"], "local_client_validation_failed");
        let unknown = LocalError::unavailable("The result is unknown; refresh.").into_response();
        let bytes = axum::body::to_bytes(unknown.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body.get("mutationOutcome").is_none());
        assert!(body.get("credential").is_none());
        assert!(require_mutation(&HeaderMap::new()).is_err());
    }

    #[test]
    fn configuration_only_references_the_private_credential_file() {
        let mut client = LocalClientView::unavailable(&input(), "test");
        client.principal_id = "client:local:server-random".to_owned();
        client.status = "ready";
        let config = render_config(
            Path::new("/PCP/bin/pcp-mcp"),
            Path::new("/PCP/local-clients/id/enrollment.json"),
            &client,
        );
        let servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        let launch = &servers[&format!("pcp-{}", input().operation_id)];
        assert_eq!(launch["command"], "/PCP/bin/pcp-mcp");
        let env = launch["env"].as_object().unwrap();
        assert_eq!(env.len(), 4);
        assert_eq!(env["PCP_MCP_SURFACE"], "generic");
        assert_eq!(env["PCP_MCP_TOOLSET"], "context");
        assert!(!env.contains_key("PCP_RUNTIME_SOCKET"));
        assert!(!env.contains_key("PCP_ALLOWED_SCOPES"));
        assert!(!env.contains_key("PCP_ACCESS_MODE"));
    }

    #[test]
    fn runtime_views_must_match_principal_and_the_complete_reviewed_grant() {
        let info = LocalEnrollmentInfo {
            principal_id: "client:local:test".to_owned(),
            principal_name: Some(input().display_name),
            requested_access: input().access(),
            identity_id: "test".to_owned(),
            request_id: None,
            registration_id: None,
        };
        let mut pending = PendingEnrollmentView {
            request_id: "req_test".to_owned(),
            client: pcp_rpc::EnrollmentClientClaim {
                principal: pcp_rpc::EnrollmentPrincipalClaim {
                    principal_id: info.principal_id.clone(),
                    principal_type: AccessPrincipalType::ModelClient,
                    display_name: info.principal_name.clone(),
                },
            },
            requested_access: input().access(),
            requested_at: String::new(),
            expires_at: String::new(),
        };
        assert!(matches_pending(&info, &input(), &pending));
        pending.requested_access.read_all_scopes = true;
        assert!(!matches_pending(&info, &input(), &pending));
        pending.requested_access = input().access();
        pending.client.principal.principal_id = "client:other".to_owned();
        assert!(!matches_pending(&info, &input(), &pending));
    }
}
