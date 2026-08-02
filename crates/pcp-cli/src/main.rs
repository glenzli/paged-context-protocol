use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use pcp_client::{EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPrincipal, AccessPrincipalType, AccessSession, Projection, ReadPage, ReadPagesRequest,
    SearchFilters, SearchMode, SearchPagesRequest,
};
use pcp_rpc::RemotePcpClient;
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".to_owned());
    let path = env::var_os("PCP_STORE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/context.sqlite3"));
    if command == "help" || command == "--help" || command == "-h" {
        print_help();
        return Ok(());
    }

    let (client, source): (Arc<dyn PcpApi>, String) =
        if let Some(socket_path) = env::var_os("PCP_RUNTIME_SOCKET") {
            let socket_path = PathBuf::from(socket_path);
            let remote = match env::var("PCP_CLIENT_ID") {
                Ok(expected_principal) => {
                    RemotePcpClient::connect_expected(&socket_path, &expected_principal).await?
                }
                Err(_) => RemotePcpClient::connect(&socket_path).await?,
            };
            (
                Arc::new(remote),
                format!("runtime:{}", socket_path.display()),
            )
        } else {
            let store = Arc::new(
                SqlitePcpStore::open(path.clone())
                    .await
                    .with_context(|| format!("open PCP Store {}", path.display()))?,
            );
            let scopes = store.local_scope_names().await?;
            let access = AccessSession::read_only(
                AccessPrincipal {
                    principal_id: "cli:pcp".to_owned(),
                    principal_type: AccessPrincipalType::Cli,
                    display_name: Some("PCP CLI".to_owned()),
                },
                format!("pcp-cli:{}", std::process::id()),
                scopes,
            );
            let store: Arc<dyn PcpStore> = store;
            (
                EmbeddedPcpClient::shared(store, access),
                format!("sqlite:{}", path.display()),
            )
        };
    let (available_scopes, _) = client.list_scopes(Vec::new(), None, 10_000, None).await?;
    let scopes = available_scopes
        .into_iter()
        .map(|scope| scope.namespace)
        .collect::<Vec<_>>();
    match command.as_str() {
        "describe" => print_json(&json!({
            "ownerId": client.owner_id(),
            "capabilities": client.capabilities(),
            "access": client.access(),
            "source": source
        }))?,
        "scopes" => {
            let query = arguments.next();
            let (items, next_cursor) = client.list_scopes(Vec::new(), query, 100, None).await?;
            print_json(&json!({"scopes": items, "nextCursor": next_cursor}))?;
        }
        "search" => {
            let query = arguments.next().context("pcp search requires a query")?;
            let mode = arguments
                .next()
                .map(|value| parse_mode(&value))
                .transpose()?
                .unwrap_or(SearchMode::Auto);
            let result = client
                .search_pages(SearchPagesRequest {
                    query,
                    scopes,
                    mode,
                    term_match: pcp_core::SearchTermMatch::All,
                    projections: pcp_core::default_search_projections(),
                    filters: SearchFilters::default(),
                    limit: 20,
                    cursor: None,
                })
                .await?;
            print_json(&result)?;
        }
        "read" => {
            let revision_id = arguments
                .next()
                .context("pcp read requires a revision id")?;
            let pages = client
                .read_pages(ReadPagesRequest {
                    revision_ids: vec![revision_id],
                    projections: vec![
                        Projection::Manifest,
                        Projection::Summary,
                        Projection::Validity,
                        Projection::Payload,
                        Projection::Sources,
                        Projection::Provenance,
                        Projection::Facets,
                        Projection::Relations,
                        Projection::History,
                    ],
                    max_chars: 64_000,
                })
                .await?;
            print_json(&json!({"pages": pages}))?;
        }
        "export" => {
            let pages = export_pages(client.as_ref(), scopes).await?;
            print_json(&json!({
                "protocolVersion": client.capabilities().protocol_version,
                "ownerId": client.owner_id(),
                "pages": pages
            }))?;
        }
        "doctor" => {
            let integrity = client.integrity_check().await?;
            let page_count = client.page_count(Vec::new()).await?;
            let (scope_details, _) = client.list_scopes(Vec::new(), None, 100, None).await?;
            print_json(&json!({
                "source": source,
                "integrity": integrity,
                "ownerId": client.owner_id(),
                "scopeCount": scope_details.len(),
                "pageCount": page_count,
                "status": if integrity == "ok" { "ready" } else { "degraded" }
            }))?;
        }
        other => anyhow::bail!("unknown pcp command: {other}"),
    }
    Ok(())
}

async fn export_pages(store: &dyn PcpApi, scopes: Vec<String>) -> Result<Vec<ReadPage>> {
    let mut cursor = None;
    let mut revision_ids = Vec::new();
    loop {
        let result = store
            .search_pages(SearchPagesRequest {
                query: String::new(),
                scopes: scopes.clone(),
                mode: SearchMode::Temporal,
                term_match: pcp_core::SearchTermMatch::All,
                projections: vec![Projection::Payload, Projection::Facets],
                filters: SearchFilters::default(),
                limit: 50,
                cursor: cursor.clone(),
            })
            .await?;
        revision_ids.extend(result.hits.into_iter().map(|hit| hit.revision_id));
        cursor = result.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let mut pages = Vec::new();
    for chunk in revision_ids.chunks(20) {
        pages.extend(
            store
                .read_pages(ReadPagesRequest {
                    revision_ids: chunk.to_vec(),
                    projections: vec![
                        Projection::Manifest,
                        Projection::Summary,
                        Projection::Validity,
                        Projection::Payload,
                        Projection::Sources,
                        Projection::Provenance,
                        Projection::Facets,
                        Projection::Relations,
                        Projection::History,
                    ],
                    max_chars: 64_000,
                })
                .await?,
        );
    }
    Ok(pages)
}

fn parse_mode(value: &str) -> Result<SearchMode> {
    match value {
        "auto" => Ok(SearchMode::Auto),
        "exact" => Ok(SearchMode::Exact),
        "text" => Ok(SearchMode::Text),
        "graph" => Ok(SearchMode::Graph),
        "temporal" => Ok(SearchMode::Temporal),
        other => anyhow::bail!("unknown search mode: {other}"),
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_help() {
    println!(
        "pcp commands:\n  describe\n  scopes [query]\n  search <query> [auto|exact|text|graph|temporal]\n  read <revision-id>\n  export\n  doctor\n\nSet PCP_RUNTIME_SOCKET to use a running local runtime, or PCP_STORE_PATH for embedded SQLite."
    );
}
