use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use pcp_client::{PcpApi, PcpTenantApi};
use pcp_core::AccessPrincipalType;
use pcp_rpc::{
    BeginEnrollmentParams, EnrollmentClient, EnrollmentClientClaim, EnrollmentPrincipalClaim,
    EnrollmentResult, EnrollmentServiceIdentity, OpenEnrollmentSessionParams,
    PCP_ENROLLMENT_PROTOCOL_ID, PCP_ENROLLMENT_PROTOCOL_VERSION, RemotePcpClient, RequestedAccess,
    RequestedAccessMode,
};
use serde::{Deserialize, Serialize};

const STATE_SCHEMA: &str = "pcp.mcp.enrollment";
const STATE_SCHEMA_VERSION: &str = "20260901.1";
const DISCOVERY_SCHEMA: &str = "infra.discovery.registration";
const DISCOVERY_VERSION: &str = "20260812.1";
const LOCAL_UNIX_SOCKET_BINDING: &str = "infra.local.unix-socket";
const MAX_STATE_BYTES: u64 = 32 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct McpEnrollmentState {
    schema: String,
    schema_version: String,
    principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal_name: Option<String>,
    requested_access: RequestedAccess,
    credential: String,
    discovery_manifest_path: PathBuf,
    identity_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    registration_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct DiscoveryRegistration {
    schema: String,
    schema_version: String,
    service: EnrollmentServiceIdentity,
    offers: Vec<DiscoveryOffer>,
}

#[derive(Clone, Debug, Deserialize)]
struct DiscoveryOffer {
    protocol: String,
    protocol_versions: Vec<String>,
    binding: String,
    endpoint: String,
}

struct SelectedEnrollmentEndpoint {
    runtime_root: PathBuf,
    public_socket: PathBuf,
    service: EnrollmentServiceIdentity,
}

pub async fn run_command(command: Option<&str>) -> Result<()> {
    let state_path = required_path("PCP_ENROLLMENT_FILE")?;
    match command.unwrap_or("status") {
        "begin" => {
            anyhow::ensure!(
                !state_path.exists(),
                "PCP enrollment state already exists at {}; use `pcp-mcp enroll status`",
                state_path.display()
            );
            let mut state = state_from_env()?;
            write_state(&state_path, &state)?;
            advance_enrollment(&state_path, &mut state).await?;
        }
        "status" => {
            let mut state = read_state(&state_path)?;
            advance_enrollment(&state_path, &mut state).await?;
        }
        other => anyhow::bail!("unsupported enrollment command `{other}`; use begin or status"),
    }
    Ok(())
}

pub async fn connect(state_path: PathBuf, expected_principal: &str) -> Result<Arc<dyn PcpApi>> {
    let mut state = read_state(&state_path)?;
    anyhow::ensure!(
        state.principal_id == expected_principal,
        "PCP enrollment Principal mismatch: expected {expected_principal}, state contains {}",
        state.principal_id
    );
    if state.registration_id.is_none() {
        advance_enrollment(&state_path, &mut state).await?;
    }
    let registration_id = state
        .registration_id
        .clone()
        .context("PCP enrollment is pending user approval")?;
    let selected = select_endpoint(&state)?;
    let response = EnrollmentClient::new(&selected.public_socket)
        .open_session(OpenEnrollmentSessionParams {
            registration_id: registration_id.clone(),
            credential: state.credential.clone(),
        })
        .await
        .context("open approved PCP enrollment session")?;
    let session = match response.result {
        EnrollmentResult::Active { session } => session,
        EnrollmentResult::Pending { .. } => {
            anyhow::bail!("PCP enrollment is pending user approval")
        }
        EnrollmentResult::Rejected { .. } => {
            anyhow::bail!("PCP enrollment was rejected or revoked")
        }
    };
    anyhow::ensure!(
        session.registration_id == registration_id,
        "PCP enrollment registration mismatch"
    );
    anyhow::ensure!(
        session.service == selected.service,
        "PCP enrollment discovery identity changed"
    );
    anyhow::ensure!(
        session.binding == LOCAL_UNIX_SOCKET_BINDING,
        "unsupported PCP enrollment binding"
    );
    anyhow::ensure!(
        session.access.principal.principal_id == expected_principal,
        "PCP enrollment returned the wrong Principal"
    );
    let socket_path = resolve_socket(&selected.runtime_root, &session.endpoint)?;
    let remote = RemotePcpClient::connect_expected(&socket_path, expected_principal)
        .await
        .context("connect approved PCP enrollment session")?;
    anyhow::ensure!(
        remote.identity_id() == selected.service.instance_id,
        "PCP RPC identity does not match enrollment discovery"
    );
    anyhow::ensure!(
        remote.access() == &session.access,
        "PCP RPC access descriptor does not match enrollment session"
    );
    Ok(Arc::new(remote))
}

async fn advance_enrollment(state_path: &Path, state: &mut McpEnrollmentState) -> Result<()> {
    let selected = select_endpoint(state)?;
    let client = EnrollmentClient::new(&selected.public_socket);
    let response = if let Some(request_id) = state.request_id.clone() {
        client
            .status(pcp_rpc::EnrollmentStatusParams {
                request_id,
                credential: state.credential.clone(),
            })
            .await
            .context("read PCP enrollment status")?
    } else {
        client
            .begin(BeginEnrollmentParams {
                client: EnrollmentClientClaim {
                    principal: EnrollmentPrincipalClaim {
                        principal_id: state.principal_id.clone(),
                        principal_type: AccessPrincipalType::ModelClient,
                        display_name: state.principal_name.clone(),
                    },
                },
                requested_access: state.requested_access.clone(),
                credential: state.credential.clone(),
            })
            .await
            .context("begin PCP enrollment")?
    };
    match response.result {
        EnrollmentResult::Pending {
            request_id,
            expires_at,
            ..
        } => {
            state.request_id = Some(request_id.clone());
            write_state(state_path, state)?;
            println!("PCP enrollment request {request_id} is pending approval until {expires_at}");
        }
        EnrollmentResult::Active { session } => {
            anyhow::ensure!(
                session.service == selected.service,
                "PCP enrollment discovery identity changed"
            );
            anyhow::ensure!(
                session.access.principal.principal_id == state.principal_id,
                "PCP enrollment returned the wrong Principal"
            );
            state.registration_id = Some(session.registration_id.clone());
            write_state(state_path, state)?;
            println!("PCP enrollment {} is active", session.registration_id);
        }
        EnrollmentResult::Rejected { request_id } => {
            anyhow::bail!("PCP enrollment request {request_id} was rejected")
        }
    }
    Ok(())
}

fn state_from_env() -> Result<McpEnrollmentState> {
    let principal_id = env::var("PCP_CLIENT_ID").context("PCP_CLIENT_ID is required")?;
    anyhow::ensure!(
        !principal_id.trim().is_empty(),
        "PCP_CLIENT_ID must not be empty"
    );
    let requested_access = RequestedAccess {
        mode: match env::var("PCP_ACCESS_MODE").as_deref().unwrap_or("read") {
            "read" => RequestedAccessMode::Read,
            "contribute" => RequestedAccessMode::Contribute,
            other => {
                anyhow::bail!("PCP MCP enrollment supports only read or contribute, not {other}")
            }
        },
        scopes: requested_scopes()?,
        read_all_scopes: requested_read_all_scopes()?,
        allow_cross_scope_derivation: false,
    };
    let discovery_manifest_path = required_path("PCP_DISCOVERY_MANIFEST")?;
    let registration = read_manifest(&discovery_manifest_path)?;
    anyhow::ensure!(
        registration.service.kind == "pcp",
        "discovery manifest is not PCP"
    );
    Ok(McpEnrollmentState {
        schema: STATE_SCHEMA.to_owned(),
        schema_version: STATE_SCHEMA_VERSION.to_owned(),
        principal_id,
        principal_name: env::var("PCP_CLIENT_NAME").ok(),
        requested_access,
        credential: random_credential()?,
        discovery_manifest_path,
        identity_id: registration.service.instance_id,
        request_id: None,
        registration_id: None,
    })
}

fn requested_scopes() -> Result<Vec<String>> {
    requested_scope_list("PCP_ALLOWED_SCOPES", true)
}

fn requested_read_all_scopes() -> Result<bool> {
    match env::var("PCP_READ_ALL_SCOPES")
        .as_deref()
        .unwrap_or("false")
    {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => anyhow::bail!("PCP_READ_ALL_SCOPES must be true, false, 1, or 0, not {other}"),
    }
}

fn requested_scope_list(name: &str, required: bool) -> Result<Vec<String>> {
    let value = env::var(name)
        .with_context(|| format!("{name} must explicitly name at least one Scope"))?;
    let scopes = value
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    anyhow::ensure!(!required || !scopes.is_empty(), "{name} must not be empty");
    Ok(scopes)
}

fn select_endpoint(state: &McpEnrollmentState) -> Result<SelectedEnrollmentEndpoint> {
    let registration = read_manifest(&state.discovery_manifest_path)?;
    anyhow::ensure!(
        registration.schema == DISCOVERY_SCHEMA,
        "unsupported Infra Discovery schema"
    );
    anyhow::ensure!(
        registration.schema_version == DISCOVERY_VERSION,
        "unsupported Infra Discovery version"
    );
    anyhow::ensure!(
        registration.service.kind == "pcp",
        "discovery manifest is not PCP"
    );
    anyhow::ensure!(
        registration.service.instance_id == state.identity_id,
        "PCP discovery identity does not match enrollment state"
    );
    let offer = registration
        .offers
        .iter()
        .find(|offer| {
            offer.protocol == PCP_ENROLLMENT_PROTOCOL_ID
                && offer
                    .protocol_versions
                    .iter()
                    .any(|version| version == PCP_ENROLLMENT_PROTOCOL_VERSION)
                && offer.binding == LOCAL_UNIX_SOCKET_BINDING
        })
        .context("PCP discovery manifest has no compatible enrollment offer")?;
    let runtime_root = state
        .discovery_manifest_path
        .parent()
        .and_then(Path::parent)
        .context("PCP discovery manifest is not inside a runtime registration directory")?
        .to_path_buf();
    validate_private_directory(&runtime_root)?;
    let public_socket = resolve_socket(&runtime_root, &offer.endpoint)?;
    Ok(SelectedEnrollmentEndpoint {
        runtime_root,
        public_socket,
        service: registration.service,
    })
}

fn read_state(path: &Path) -> Result<McpEnrollmentState> {
    validate_private_file(path, MAX_STATE_BYTES, "PCP MCP enrollment state")?;
    let state: McpEnrollmentState = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read PCP enrollment state {}", path.display()))?,
    )
    .context("decode PCP MCP enrollment state")?;
    anyhow::ensure!(
        state.schema == STATE_SCHEMA && state.schema_version == STATE_SCHEMA_VERSION,
        "unsupported PCP MCP enrollment state"
    );
    validate_credential(&state.credential)?;
    Ok(state)
}

fn read_manifest(path: &Path) -> Result<DiscoveryRegistration> {
    validate_private_file(path, MAX_MANIFEST_BYTES, "Infra Discovery registration")?;
    serde_json::from_slice(
        &fs::read(path)
            .with_context(|| format!("read Infra Discovery registration {}", path.display()))?,
    )
    .context("decode Infra Discovery registration")
}

fn write_state(path: &Path, state: &McpEnrollmentState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create PCP enrollment directory {}", parent.display()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        validate_private_directory(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("PCP enrollment state path has no file name")?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(state).context("encode PCP MCP enrollment state")?;
    bytes.push(b'\n');
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_STATE_BYTES,
        "PCP enrollment state is too large"
    );
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| {
            format!(
                "create temporary PCP enrollment state {}",
                temporary.display()
            )
        })?;
    let write_result = (|| -> Result<()> {
        output.write_all(&bytes)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temporary, path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.with_context(|| format!("persist PCP enrollment state {}", path.display()))
}

fn resolve_socket(runtime_root: &Path, endpoint: &str) -> Result<PathBuf> {
    let path = resolve_endpoint(runtime_root, endpoint)?;
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("inspect PCP enrollment socket {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_socket() && !metadata.file_type().is_symlink(),
        "PCP enrollment endpoint is not a real Unix socket: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "PCP enrollment socket must be mode 0600: {}",
        path.display()
    );
    Ok(path)
}

fn resolve_endpoint(runtime_root: &Path, endpoint: &str) -> Result<PathBuf> {
    let path = Path::new(endpoint);
    let mut components = path.components();
    anyhow::ensure!(
        matches!(components.next(), Some(Component::Normal(value)) if value == "sockets"),
        "PCP enrollment endpoint must be inside sockets/"
    );
    let opaque = match components.next() {
        Some(Component::Normal(value)) => value.to_str().context("PCP endpoint is not UTF-8")?,
        _ => anyhow::bail!("PCP enrollment endpoint has no opaque socket name"),
    };
    anyhow::ensure!(
        components.next().is_none(),
        "PCP enrollment endpoint is not canonical"
    );
    let opaque = opaque
        .strip_suffix(".sock")
        .context("PCP enrollment endpoint must end in .sock")?;
    anyhow::ensure!(
        !opaque.is_empty()
            && opaque.len() <= 16
            && opaque
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
        "PCP enrollment endpoint has an invalid opaque ID"
    );
    Ok(runtime_root.join(endpoint))
}

fn validate_private_file(path: &Path, max_bytes: u64, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{label} is not a regular file"
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "{label} must be mode 0600"
    );
    anyhow::ensure!(metadata.len() <= max_bytes, "{label} is too large");
    Ok(())
}

fn validate_private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect private directory {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "private path is not a real directory"
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o700,
        "private directory must be mode 0700"
    );
    Ok(())
}

fn required_path(name: &str) -> Result<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .with_context(|| format!("{name} is required"))
}

fn random_credential() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .context("open system random source")?
        .read_exact(&mut bytes)
        .context("read enrollment credential entropy")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn validate_credential(value: &str) -> Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "PCP enrollment credential is invalid"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_endpoint;
    use std::path::Path;

    #[test]
    fn enrollment_endpoint_is_bounded_to_runtime_sockets() {
        let root = Path::new("/private/runtime/infra-protocol");
        assert_eq!(
            resolve_endpoint(root, "sockets/ABC123.sock").expect("valid endpoint"),
            root.join("sockets/ABC123.sock")
        );
        assert!(resolve_endpoint(root, "../escape.sock").is_err());
        assert!(resolve_endpoint(root, "sockets/too/many.sock").is_err());
        assert!(resolve_endpoint(root, "sockets/invalid!.sock").is_err());
    }
}
