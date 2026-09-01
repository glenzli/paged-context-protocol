use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::{AccessMode, EmbeddedPcpClient, PcpApi};
use pcp_core::{AccessPrincipal, AccessPrincipalType};
use pcp_mcp::PcpMcpServer;
use pcp_rpc::RemotePcpClient;
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;
use rmcp::{ServiceExt, transport::stdio};

mod enrollment;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() == Some("enroll") {
        enrollment::run_command(args.next().as_deref()).await?;
        return Ok(());
    }
    let client: Arc<dyn PcpApi> = if let Some(state_path) = env::var_os("PCP_ENROLLMENT_FILE") {
        let expected_principal = env::var("PCP_CLIENT_ID")
            .context("PCP_CLIENT_ID must match the enrolled Runtime Principal")?;
        enrollment::connect(PathBuf::from(state_path), &expected_principal).await?
    } else if let Some(socket_path) = env::var_os("PCP_RUNTIME_SOCKET") {
        let expected_principal = env::var("PCP_CLIENT_ID")
            .context("PCP_CLIENT_ID must match the configured runtime endpoint")?;
        Arc::new(
            RemotePcpClient::connect_expected(PathBuf::from(socket_path), &expected_principal)
                .await
                .context("connect PCP runtime")?,
        )
    } else {
        embedded_client().await?
    };
    let service = PcpMcpServer::new(client)
        .serve(stdio())
        .await
        .context("start PCP MCP stdio server")?;
    service.waiting().await.context("run PCP MCP server")?;
    Ok(())
}

async fn embedded_client() -> Result<Arc<dyn PcpApi>> {
    let path = env::var_os("PCP_STORE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/context.sqlite3"));
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
    let principal_id =
        env::var("PCP_CLIENT_ID").context("PCP_CLIENT_ID must identify this MCP access surface")?;
    anyhow::ensure!(
        !principal_id.trim().is_empty(),
        "PCP_CLIENT_ID must not be empty"
    );
    let access_mode = env::var("PCP_ACCESS_MODE")
        .unwrap_or_else(|_| "read".to_owned())
        .parse::<AccessMode>()?;
    let allow_cross_scope_derivation = env::var("PCP_ALLOW_CROSS_SCOPE_DERIVATION")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    let session_id = env::var("PCP_SESSION_ID").unwrap_or_else(|_| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("pcp-mcp:{}:{started}", std::process::id())
    });
    let access = access_mode.session(
        AccessPrincipal {
            principal_id,
            principal_type: AccessPrincipalType::ModelClient,
            display_name: env::var("PCP_CLIENT_NAME").ok(),
        },
        session_id,
        scopes,
        allow_cross_scope_derivation,
    );
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", path.display()))?,
    );
    Ok(EmbeddedPcpClient::shared(store, access))
}
