use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::{AccessMode, EmbeddedPcpClient};
use pcp_core::{AccessPrincipal, AccessPrincipalType};
use pcp_runtime::serve_unix;
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

#[tokio::main]
async fn main() -> Result<()> {
    let store_path = env::var_os("PCP_STORE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/context.sqlite3"));
    let socket_path = env::var_os("PCP_RUNTIME_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/pcp-runtime.sock"));
    let scopes = required_scopes()?;
    let principal_id =
        env::var("PCP_CLIENT_ID").context("PCP_CLIENT_ID must identify this runtime endpoint")?;
    anyhow::ensure!(
        !principal_id.trim().is_empty(),
        "PCP_CLIENT_ID must not be empty"
    );
    let access_mode = env::var("PCP_ACCESS_MODE")
        .unwrap_or_else(|_| "read".to_owned())
        .parse::<AccessMode>()?;
    let principal_type = env::var("PCP_CLIENT_TYPE")
        .ok()
        .map(|value| parse_principal_type(&value))
        .transpose()?
        .unwrap_or(AccessPrincipalType::Service);
    let allow_cross_scope_derivation = env::var("PCP_ALLOW_CROSS_SCOPE_DERIVATION")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let access = access_mode.session(
        AccessPrincipal {
            principal_id,
            principal_type,
            display_name: env::var("PCP_CLIENT_NAME").ok(),
        },
        env::var("PCP_SESSION_ID")
            .unwrap_or_else(|_| format!("pcp-runtime:{}:{started}", std::process::id())),
        scopes,
        allow_cross_scope_derivation,
    );
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(store_path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", store_path.display()))?,
    );
    let client = EmbeddedPcpClient::shared(store, access);
    serve_unix(socket_path, client).await
}

fn required_scopes() -> Result<Vec<String>> {
    let scopes = env::var("PCP_ALLOWED_SCOPES")
        .context("PCP_ALLOWED_SCOPES must explicitly name at least one Scope")?
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !scopes.is_empty(),
        "PCP_ALLOWED_SCOPES must explicitly name at least one Scope"
    );
    Ok(scopes)
}

fn parse_principal_type(value: &str) -> Result<AccessPrincipalType> {
    match value {
        "host" => Ok(AccessPrincipalType::Host),
        "model_client" => Ok(AccessPrincipalType::ModelClient),
        "cli" => Ok(AccessPrincipalType::Cli),
        "service" => Ok(AccessPrincipalType::Service),
        other => anyhow::bail!("unsupported PCP_CLIENT_TYPE: {other}"),
    }
}
