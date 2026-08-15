use std::{env, io::Read, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use pcp_client::{AccessMode, EmbeddedPcpClient, PcpApi};
use pcp_core::{
    AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
    CollectRevisionRetentionRequest, LifecycleStatus, PagePayload, PlanRevisionRetentionRequest,
    Projection, ReadPage, ReadPagesRequest, RetentionPolicy, RevisePageRequest, SearchFilters,
    SearchMode, SearchPagesRequest,
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
            let principal = AccessPrincipal {
                principal_id: "cli:pcp".to_owned(),
                principal_type: AccessPrincipalType::Cli,
                display_name: Some("PCP CLI".to_owned()),
            };
            let session_id = format!("pcp-cli:{}", std::process::id());
            let access = match command.as_str() {
                "retention-plan" => AccessMode::Audit.session(principal, session_id, scopes, false),
                "retention-collect" | "revise" => {
                    AccessMode::Admin.session(principal, session_id, scopes, false)
                }
                _ => AccessSession::read_only(principal, session_id, scopes),
            };
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
            "identityId": client.identity_id(),
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
            let page_id = arguments.next().context("pcp read requires a Page ID")?;
            let pages = client
                .read_pages(ReadPagesRequest {
                    page_ids: vec![page_id],
                    revision_ids: Vec::new(),
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
                "identityId": client.identity_id(),
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
                "identityId": client.identity_id(),
                "scopeCount": scope_details.len(),
                "pageCount": page_count,
                "status": if integrity == "ok" { "ready" } else { "degraded" }
            }))?;
        }
        "retention-plan" => {
            let minimum_age_days = parse_optional_u32(
                arguments.next(),
                "minimum age days",
                RetentionPolicy::default().minimum_age_days,
            )?;
            let keep_recent_revisions_per_page = parse_optional_u32(
                arguments.next(),
                "recent revisions per Page",
                RetentionPolicy::default().keep_recent_revisions_per_page,
            )?;
            let sample_limit = parse_optional_u32(
                arguments.next(),
                "sample limit",
                RetentionPolicy::default().sample_limit,
            )?;
            let plan = client
                .plan_revision_retention(PlanRevisionRetentionRequest {
                    scopes,
                    policy: RetentionPolicy {
                        minimum_age_days,
                        keep_recent_revisions_per_page,
                        sample_limit,
                    },
                })
                .await?;
            print_json(&plan)?;
        }
        "retention-collect" => {
            anyhow::ensure!(
                arguments.next().as_deref() == Some("--confirm"),
                "pcp retention-collect requires --confirm before policy arguments"
            );
            let minimum_age_days = parse_optional_u32(
                arguments.next(),
                "minimum age days",
                RetentionPolicy::default().minimum_age_days,
            )?;
            let keep_recent_revisions_per_page = parse_optional_u32(
                arguments.next(),
                "recent revisions per Page",
                RetentionPolicy::default().keep_recent_revisions_per_page,
            )?;
            let sample_limit = parse_optional_u32(
                arguments.next(),
                "sample limit",
                RetentionPolicy::default().sample_limit,
            )?
            .clamp(1, 500);
            let policy = RetentionPolicy {
                minimum_age_days,
                keep_recent_revisions_per_page,
                sample_limit,
            };
            let plan = client
                .plan_revision_retention(PlanRevisionRetentionRequest {
                    scopes: scopes.clone(),
                    policy: policy.clone(),
                })
                .await?;
            anyhow::ensure!(
                !plan.candidates_truncated,
                "retention plan has more than {sample_limit} candidates; collect in smaller explicit batches"
            );
            anyhow::ensure!(
                !plan.candidates.is_empty(),
                "retention plan has no eligible Revision candidates"
            );
            let result = client
                .collect_revision_retention(CollectRevisionRetentionRequest {
                    scopes,
                    policy,
                    revision_ids: plan
                        .candidates
                        .into_iter()
                        .map(|candidate| candidate.revision_id)
                        .collect(),
                })
                .await?;
            print_json(&result)?;
        }
        "revise" => {
            anyhow::ensure!(
                arguments.next().as_deref() == Some("--confirm"),
                "pcp revise requires --confirm"
            );
            let page_id = arguments.next().context("pcp revise requires a Page ID")?;
            let page = client
                .read_pages(ReadPagesRequest {
                    page_ids: vec![page_id],
                    revision_ids: Vec::new(),
                    projections: vec![
                        Projection::Manifest,
                        Projection::Payload,
                        Projection::Sources,
                        Projection::Facets,
                    ],
                    max_chars: 256_000,
                })
                .await?
                .into_iter()
                .next()
                .context("revision target Page could not be read")?;
            let mut content = String::new();
            std::io::stdin()
                .read_to_string(&mut content)
                .context("read revised Page content from stdin")?;
            anyhow::ensure!(
                !content.trim().is_empty(),
                "pcp revise requires the new Markdown payload on stdin"
            );
            let result = client
                .revise_page(RevisePageRequest {
                    page_id: page.page.page_id,
                    expected_revision_id: page.revision.revision_id.clone(),
                    created_by: Actor {
                        actor_type: ActorType::Tool,
                        actor_id: "cli:pcp".to_owned(),
                    },
                    lifecycle_status: LifecycleStatus::Active,
                    observed_at: None,
                    valid_from: page.revision.valid_from,
                    valid_to: page.revision.valid_to,
                    payload: Some(PagePayload {
                        media_type: page
                            .revision
                            .payload
                            .as_ref()
                            .map(|payload| payload.media_type.clone())
                            .unwrap_or_else(|| "text/markdown".to_owned()),
                        content,
                    }),
                    source_refs: page.revision.source_refs,
                    facets: page.revision.facets,
                    provenance: Vec::new(),
                    initial_relations: Vec::new(),
                    idempotency_key: Some(format!("cli:revise:{}", page.revision.revision_id)),
                })
                .await?;
            print_json(&result)?;
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
                    page_ids: Vec::new(),
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

fn parse_optional_u32(value: Option<String>, name: &str, default: u32) -> Result<u32> {
    value
        .map(|value| {
            value
                .parse::<u32>()
                .with_context(|| format!("invalid {name}: {value}"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn print_help() {
    println!(
        "pcp commands:\n  describe\n  scopes [query]\n  search <query> [auto|exact|text|graph|temporal]\n  read <page-id>\n  export\n  doctor\n  retention-plan [minimum-age-days] [keep-recent-per-page] [sample-limit]\n  retention-collect --confirm [minimum-age-days] [keep-recent-per-page] [sample-limit]\n  revise --confirm <page-id> < revised.md\n\nSet PCP_RUNTIME_SOCKET to use a running local runtime, or PCP_STORE_PATH for embedded SQLite."
    );
}
