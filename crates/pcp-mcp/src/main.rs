use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_core::{AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, ScopeGrant};
use pcp_mcp::PcpMcpServer;
use pcp_sqlite::SqlitePcpStore;
use pcp_store::{PcpClient, PcpStore};
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<()> {
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
    let access_mode = env::var("PCP_ACCESS_MODE").unwrap_or_else(|_| "read".to_owned());
    let permissions = permissions_for_mode(&access_mode)?;
    let allow_cross_scope_derivation = env::var("PCP_ALLOW_CROSS_SCOPE_DERIVATION")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    let grants = scopes
        .into_iter()
        .map(|namespace| {
            let mut permissions = permissions.clone();
            if allow_cross_scope_derivation {
                permissions.push(AccessPermission::DeriveAcrossScopes);
            }
            ScopeGrant {
                namespace,
                permissions,
            }
        })
        .collect();
    let session_id = env::var("PCP_SESSION_ID").unwrap_or_else(|_| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("pcp-mcp:{}:{started}", std::process::id())
    });
    let access = AccessSession::new(
        AccessPrincipal {
            principal_id,
            principal_type: AccessPrincipalType::ModelClient,
            display_name: env::var("PCP_CLIENT_NAME").ok(),
        },
        session_id,
        grants,
    );
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", path.display()))?,
    );
    let service = PcpMcpServer::new(PcpClient::new(store, access))
        .serve(stdio())
        .await
        .context("start PCP MCP stdio server")?;
    service.waiting().await.context("run PCP MCP server")?;
    Ok(())
}

fn permissions_for_mode(mode: &str) -> Result<Vec<AccessPermission>> {
    let mut permissions = vec![
        AccessPermission::ListScopes,
        AccessPermission::Search,
        AccessPermission::ReadSummary,
        AccessPermission::ReadDetail,
    ];
    match mode {
        "read" => {}
        "write" | "admin" => permissions.extend([
            AccessPermission::Write,
            AccessPermission::Revise,
            AccessPermission::Summarize,
            AccessPermission::Link,
            AccessPermission::Assess,
        ]),
        other => anyhow::bail!("unsupported PCP_ACCESS_MODE: {other}"),
    }
    if mode == "admin" {
        permissions.extend([
            AccessPermission::ManageScope,
            AccessPermission::Retract,
            AccessPermission::Audit,
        ]);
    }
    Ok(permissions)
}
