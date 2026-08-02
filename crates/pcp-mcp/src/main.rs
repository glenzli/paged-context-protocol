use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use pcp_mcp::PcpMcpServer;
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<()> {
    let path = env::var_os("PCP_STORE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/context.sqlite3"));
    let restricted_scopes = env::var("PCP_ALLOWED_SCOPES").ok().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", path.display()))?,
    );
    let service = PcpMcpServer::new(store, restricted_scopes)
        .serve(stdio())
        .await
        .context("start PCP MCP stdio server")?;
    service.waiting().await.context("run PCP MCP server")?;
    Ok(())
}
