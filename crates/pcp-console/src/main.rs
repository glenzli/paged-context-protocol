use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::Read,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use pcp_client::{ContentLibraryResult, PcpApi, PcpTenantApi};
use pcp_core::{
    Actor, ActorType, ArchivePageRequest, BrowseIndexOrder, IntentEffort, LifecycleStatus,
    PackPagesRequest, PagePayload, PageRevisionRef, PlanRevisionRetentionRequest, Projection,
    QueryContextRequest, ReadPage, ReadPagesRequest, Relation, RestoreArchivedPageRequest,
    RetentionPolicy, SearchFilters, SearchHit, SearchMode, SearchPagesRequest, SourceRef,
    SourceSpan, UnpackPageRequest,
};
use pcp_rpc::{EnrollmentAdminClient, EnrollmentAdminResponse, RemotePcpClient};
use pcp_runtime::{
    AnalyzeMaintenanceArchiveRequest, AnalyzeMaintenancePacksRequest,
    AnalyzeMaintenanceRelationRequest, AnalyzeMaintenanceSummariesRequest,
    AnalyzeMaintenanceSummaryRequest, AnalyzeMaintenanceTopicRequest, ApplyMaintenancePackRequest,
    ApplyMaintenanceRelationRequest, ApplyMaintenanceSummaryRequest, ApplyMaintenanceTopicRequest,
    MaintenanceMode, MaintenanceOperator, MaintenanceReviewPayload, MaintenanceReviewStatus,
    RuntimeConfig, RuntimeMaintainer,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, time::Instant};
use url::Url;

const DEFAULT_BIND: &str = "127.0.0.1:4318";
const DEFAULT_PRINCIPAL_ID: &str = "operator:local";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_PAGE_LIMIT: u32 = 20;
const MAX_PAGE_LIMIT: u32 = 50;
// The Console offers up to 30 rows at once. Reserve enough preview budget for
// 30 bounded 700-character snippets so the Store can fulfill that selection.
const PAGE_LIST_PREVIEW_CHARS: u32 = 32_000;
const MAX_HISTORY_REVISIONS: usize = 20;
const MAX_RETENTION_SAMPLE_LIMIT: u32 = 100;
const MAX_LOCAL_MEDIA_BYTES: u64 = 32 * 1024 * 1024;
const LOCAL_MEDIA_ROOTS_ENV: &str = "PCP_CONSOLE_LOCAL_MEDIA_ROOTS";
const CONSOLE_STATIC_CACHE_CONTROL: &str = "no-store";

mod graph_view;
mod managed;

#[derive(Clone)]
struct AppState {
    client: Arc<RemotePcpClient>,
    enrollment: EnrollmentAdminClient,
    runtime: Option<Arc<managed::ManagedRuntime>>,
    runtime_config: Option<PathBuf>,
    local_media_roots: Arc<Vec<PathBuf>>,
}

#[derive(Debug)]
struct ApiError(anyhow::Error);

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error = format!("{:#}", self.0);
        eprintln!("PCP Console request failed: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response()
    }
}

#[derive(Default, Deserialize)]
struct PageQuery {
    q: Option<String>,
    scope: Option<String>,
    order: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Default, Deserialize)]
struct GovernancePageQuery {
    scope: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GovernanceMutationRequest {
    page_id: String,
    expected_revision_id: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsolePageResult {
    hits: Vec<ConsolePageHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
    total_pages: u64,
    total_content_chars: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsolePageHit {
    #[serde(flatten)]
    hit: SearchHit,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_payload: Option<PagePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_stats: Option<PageListRelationStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_span: Option<SourceSpan>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsolePagePreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_payload: Option<PagePayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageListRelationStats {
    total: usize,
    incoming: usize,
    outgoing: usize,
}

impl PageListRelationStats {
    fn from_relations(page_id: &str, relations: &[Relation]) -> Self {
        let incoming = relations
            .iter()
            .filter(|relation| relation.to_page_id == page_id)
            .count();
        let outgoing = relations
            .iter()
            .filter(|relation| relation.from_page_id == page_id)
            .count();
        Self {
            total: relations
                .iter()
                .filter(|relation| {
                    relation.from_page_id == page_id || relation.to_page_id == page_id
                })
                .count(),
            incoming,
            outgoing,
        }
    }
}

struct PageListMetadata {
    preview_payload: Option<PagePayload>,
    relation_stats: PageListRelationStats,
    source_span: Option<SourceSpan>,
}

#[derive(Deserialize)]
struct LocalSourceLocator {
    uri: String,
}

#[derive(Default, Deserialize)]
struct AccessQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntentQueryRequest {
    query: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    result_limit: Option<u32>,
    #[serde(default)]
    context_budget_chars: Option<u32>,
    #[serde(default)]
    intent_effort: IntentEffort,
}

#[derive(Default, Deserialize)]
struct GraphQuery {
    depth: Option<usize>,
    limit: Option<usize>,
}

#[derive(Default, Deserialize)]
struct HealthQuery {
    scope: Option<String>,
    hours: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetentionQuery {
    scope: Option<String>,
    minimum_age_days: Option<u32>,
    keep_recent_revisions_per_page: Option<u32>,
    sample_limit: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaintenanceScanRequest {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RelationReviewDecisionRequest {
    #[serde(default)]
    suppress: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MaintenanceSettingsRequest {
    enabled: bool,
    mode: MaintenanceMode,
    min_new_pages: usize,
    quiet_period_seconds: u64,
    max_wait_seconds: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepairPackSplitRequest {
    page_id: String,
    expected_revision_id: String,
    split_after_source_positions: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetirePackRequest {
    page_id: String,
    expected_revision_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepackRestoredPackRequest {
    pages: Vec<PageRevisionRef>,
    split_after_source_positions: Vec<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let runtime = match managed::ManagedOptions::parse(env::args_os().skip(1))? {
        Some(options) => Some(managed::ManagedRuntime::start(options).await?),
        None => None,
    };
    let socket_path = runtime
        .as_ref()
        .map(|runtime| runtime.operator_socket().to_path_buf())
        .or_else(|| env::var_os("PCP_RUNTIME_SOCKET").map(PathBuf::from))
        .context(
            "PCP_RUNTIME_SOCKET must point to the operator endpoint unless --managed is used",
        )?;
    let runtime_config = runtime
        .as_ref()
        .map(|runtime| runtime.runtime_config().to_path_buf())
        .or_else(|| env::var_os("PCP_RUNTIME_CONFIG").map(PathBuf::from));
    let principal_id =
        env::var("PCP_CLIENT_ID").unwrap_or_else(|_| DEFAULT_PRINCIPAL_ID.to_owned());
    let client = connect_runtime(&socket_path, &principal_id).await?;
    let enrollment_socket = env::var_os("PCP_ENROLLMENT_ADMIN_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            socket_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("pcp-enrollment-admin.sock")
        });
    let bind = env::var("PCP_CONSOLE_BIND")
        .unwrap_or_else(|_| DEFAULT_BIND.to_owned())
        .parse::<SocketAddr>()
        .context("parse PCP_CONSOLE_BIND")?;
    let local_media_roots = local_media_roots()?;
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind PCP Console at {bind}"))?;
    println!("PCP Console is listening at http://{bind}");
    let result = axum::serve(
        listener,
        router(AppState {
            client: Arc::new(client),
            enrollment: EnrollmentAdminClient::new(enrollment_socket),
            runtime: runtime.clone(),
            runtime_config,
            local_media_roots: Arc::new(local_media_roots),
        }),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("serve PCP Console");
    if let Some(runtime) = runtime {
        runtime.shutdown().await.context("stop PCP Runtime")?;
    }
    result
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/page-inspector.js", get(page_inspector_js))
        .route("/page-content.js", get(page_content_js))
        .route("/page-content.css", get(page_content_css))
        .route("/page-graph.js", get(page_graph_js))
        .route("/health-view.js", get(health_view_js))
        .route("/retention-view.js", get(retention_view_js))
        .route("/query-view.js", get(query_view_js))
        .route("/progressive-operation.js", get(progressive_operation_js))
        .route(
            "/maintenance-relation-decisions.js",
            get(maintenance_relation_decisions_js),
        )
        .route(
            "/maintenance-convergence.js",
            get(maintenance_convergence_js),
        )
        .route("/styles.css", get(styles_css))
        .route("/api/health", get(health))
        .route("/api/runtime", get(runtime_status))
        .route("/api/runtime/restart", post(restart_runtime))
        .route("/api/overview", get(overview))
        .route("/api/pages", get(pages))
        .route("/api/governance/pages", get(governance_pages))
        .route("/api/governance/archive", post(archive_governance_page))
        .route("/api/governance/restore", post(restore_governance_page))
        .route("/api/query/audit", get(query_audit))
        .route("/api/query/semantic-search", post(run_semantic_search))
        .route("/api/query/match-intent", post(run_match_intent))
        .route("/api/pages/{page_id}/preview", get(page_preview))
        .route("/api/pages/{page_id}", get(page_detail))
        .route("/api/pages/{page_id}/raw", get(page_raw))
        .route("/api/pages/{page_id}/media/{source_index}", get(page_media))
        .route("/api/pages/{page_id}/graph", get(page_graph))
        .route("/api/pages/{page_id}/lineage", get(page_lineage))
        .route("/api/metrics", get(health_metrics))
        .route("/api/retention", get(retention_plan))
        .route("/api/maintenance", get(maintenance_status))
        .route(
            "/api/maintenance/settings",
            post(update_maintenance_settings),
        )
        .route("/api/maintenance/scan", post(maintenance_scan))
        .route("/api/maintenance/analyze", post(maintenance_analyze))
        .route("/api/maintenance/converge", post(maintenance_converge))
        .route("/api/maintenance/reviews", get(maintenance_reviews))
        .route(
            "/api/maintenance/reviews/{candidate_id}/accept",
            post(accept_maintenance_review),
        )
        .route(
            "/api/maintenance/reviews/{candidate_id}/reject",
            post(reject_maintenance_review),
        )
        .route(
            "/api/maintenance/reviews/{candidate_id}/defer",
            post(defer_maintenance_review),
        )
        .route(
            "/api/maintenance/reviews/{candidate_id}/suppress",
            post(suppress_maintenance_review),
        )
        .route(
            "/api/maintenance/archive/scan",
            post(maintenance_archive_scan),
        )
        .route(
            "/api/maintenance/archive/analyze",
            post(maintenance_archive_analyze),
        )
        .route(
            "/api/maintenance/archive/apply",
            post(maintenance_archive_apply),
        )
        .route("/api/maintenance/packs/apply", post(maintenance_apply_pack))
        .route(
            "/api/maintenance/packs/repair-split",
            post(maintenance_repair_pack_split),
        )
        .route(
            "/api/maintenance/packs/retire",
            post(maintenance_retire_pack),
        )
        .route(
            "/api/maintenance/packs/repack-restored",
            post(maintenance_repack_restored),
        )
        .route(
            "/api/maintenance/summaries/analyze",
            post(maintenance_analyze_summary),
        )
        .route(
            "/api/maintenance/summaries/analyze-batch",
            post(maintenance_analyze_summaries),
        )
        .route(
            "/api/maintenance/summaries/apply",
            post(maintenance_apply_summary),
        )
        .route(
            "/api/maintenance/relations/analyze",
            post(maintenance_analyze_relation),
        )
        .route(
            "/api/maintenance/relations/apply",
            post(maintenance_apply_relation),
        )
        .route(
            "/api/maintenance/relations/reject",
            post(maintenance_reject_relation),
        )
        .route(
            "/api/maintenance/relations/suppress",
            post(maintenance_suppress_relation),
        )
        .route(
            "/api/maintenance/topics/analyze",
            post(maintenance_analyze_topic),
        )
        .route(
            "/api/maintenance/topics/apply",
            post(maintenance_apply_topic),
        )
        .route(
            "/api/maintenance/relation-reviews",
            get(maintenance_relation_reviews),
        )
        .route(
            "/api/maintenance/relation-reviews/{candidate_id}/approve",
            post(approve_relation_review),
        )
        .route(
            "/api/maintenance/relation-reviews/{candidate_id}/reject",
            post(reject_relation_review),
        )
        .route("/api/access", get(access_log))
        .route("/api/enrollment", get(enrollment_snapshot))
        .route(
            "/api/enrollment/requests/{request_id}/approve",
            post(approve_enrollment),
        )
        .route(
            "/api/enrollment/requests/{request_id}/reject",
            post(reject_enrollment),
        )
        .route(
            "/api/enrollment/registrations/{registration_id}/revoke",
            post(revoke_enrollment),
        )
        .with_state(state)
}

async fn connect_runtime(
    socket_path: &std::path::Path,
    principal_id: &str,
) -> Result<RemotePcpClient> {
    let started = Instant::now();
    loop {
        match RemotePcpClient::connect_expected(socket_path, principal_id).await {
            Ok(client) => return Ok(client),
            Err(error) if started.elapsed() >= CONNECT_TIMEOUT => {
                return Err(error).with_context(|| {
                    format!(
                        "connect PCP operator endpoint at {} within {} seconds",
                        socket_path.display(),
                        CONNECT_TIMEOUT.as_secs()
                    )
                });
            }
            Err(_) => tokio::time::sleep(CONNECT_RETRY_INTERVAL).await,
        }
    }
}

fn static_asset(content_type: &'static str, contents: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, CONSOLE_STATIC_CACHE_CONTROL),
        ],
        contents,
    )
        .into_response()
}

async fn index() -> Response {
    static_asset("text/html; charset=utf-8", include_str!("index.html"))
}

async fn app_js() -> Response {
    static_asset("text/javascript; charset=utf-8", include_str!("app.js"))
}

async fn progressive_operation_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("progressive-operation.js"),
    )
}

async fn maintenance_relation_decisions_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("maintenance-relation-decisions.js"),
    )
}

async fn maintenance_convergence_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("maintenance-convergence.js"),
    )
}

async fn page_inspector_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("page-inspector.js"),
    )
}

async fn page_content_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("page-content.js"),
    )
}

async fn page_content_css() -> Response {
    static_asset("text/css; charset=utf-8", include_str!("page-content.css"))
}

async fn page_graph_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("page-graph.js"),
    )
}

async fn health_view_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("health-view.js"),
    )
}

async fn retention_view_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("retention-view.js"),
    )
}

async fn query_view_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_str!("query_view.js"),
    )
}

async fn styles_css() -> Response {
    static_asset("text/css; charset=utf-8", include_str!("styles.css"))
}

async fn health(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.client.page_count(Vec::new()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn runtime_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let reachable = state.client.page_count(Vec::new()).await.is_ok();
    let managed = match &state.runtime {
        Some(runtime) => {
            let status = runtime.status().await;
            json!({
                "managed": status.managed,
                "ownsProcess": status.owns_process,
                "pid": status.pid,
                "home": status.home,
            })
        }
        None => json!({"managed": false, "ownsProcess": false}),
    };
    Ok(Json(json!({"reachable": reachable, "lifecycle": managed})))
}

async fn restart_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let runtime = state
        .runtime
        .as_ref()
        .context("PCP Console was not started in managed mode")?;
    let status = runtime.restart().await?;
    Ok(Json(json!({
        "restarted": true,
        "pid": status.pid,
        "home": status.home,
    })))
}

async fn overview(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let (integrity, scopes, content_library) = tokio::try_join!(
        state.client.integrity_check(),
        state.client.list_scopes(Vec::new(), None, 10_000, None),
        state.client.content_library_summary(Vec::new()),
    )?;
    let content_by_scope = content_library
        .scopes
        .iter()
        .map(|scope| {
            (
                scope.namespace.as_str(),
                (scope.page_count, scope.content_chars),
            )
        })
        .collect::<HashMap<_, _>>();
    let scopes = scopes
        .0
        .into_iter()
        .map(|scope| {
            let (page_count, content_chars) = content_by_scope
                .get(scope.namespace.as_str())
                .copied()
                .unwrap_or((0, 0));
            json!({
                "namespace": scope.namespace,
                "displayName": scope.display_name,
                "description": scope.description,
                "parentNamespace": scope.parent_namespace,
                "createdAt": scope.created_at,
                "updatedAt": scope.updated_at,
                "pageCount": page_count,
                "contentChars": content_chars,
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "integrity": integrity,
        "identityId": state.client.identity_id(),
        "principal": state.client.access().principal,
        "grants": state.client.access().grants,
        "capabilities": state.client.capabilities(),
        "runtime": {
            "pid": state.client.server_pid(),
            "startedAtUnixMs": state.client.server_started_at_unix_ms(),
        },
        "pageCount": content_library.page_count,
        "contentChars": content_library.content_chars,
        "scopes": scopes,
    })))
}

async fn pages(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .clamp(1, MAX_PAGE_LIMIT);
    let scopes = selected_scopes(state.client.as_ref(), query.scope.as_deref());
    let search_query = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let result = state
        .client
        .browse_content_pages(
            scopes,
            search_query,
            parse_browse_index_order(query.order.as_deref())?,
            limit,
            query.cursor,
            PAGE_LIST_PREVIEW_CHARS,
        )
        .await?;
    Ok(Json(json!(
        console_page_result(state.client.as_ref(), result).await?
    )))
}

async fn governance_pages(
    State(state): State<AppState>,
    Query(query): Query<GovernancePageQuery>,
) -> Result<Json<pcp_core::SearchResult>, ApiError> {
    let lifecycle_status = match query.status.as_deref().unwrap_or("active") {
        "active" => LifecycleStatus::Active,
        "archived" => LifecycleStatus::Archived,
        _ => {
            return Err(ApiError(anyhow::anyhow!(
                "unsupported governance Page status"
            )));
        }
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .clamp(1, MAX_PAGE_LIMIT);
    let scopes = selected_scopes(state.client.as_ref(), query.scope.as_deref());
    Ok(Json(
        state
            .client
            .search_pages(SearchPagesRequest {
                query: String::new(),
                scopes,
                mode: SearchMode::Temporal,
                term_match: Default::default(),
                projections: vec![
                    Projection::Manifest,
                    Projection::Payload,
                    Projection::Facets,
                ],
                filters: SearchFilters {
                    lifecycle_status: vec![lifecycle_status],
                    ..Default::default()
                },
                limit,
                cursor: query.cursor,
            })
            .await?,
    ))
}

async fn archive_governance_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<GovernanceMutationRequest>,
) -> Result<Json<pcp_core::PageLifecycleTransitionResult>, ApiError> {
    archive_page_for_governance(&state, &headers, request).await
}

async fn archive_page_for_governance(
    state: &AppState,
    headers: &HeaderMap,
    request: GovernanceMutationRequest,
) -> Result<Json<pcp_core::PageLifecycleTransitionResult>, ApiError> {
    require_console_mutation(headers)?;
    let reason = request.reason.trim();
    if reason.is_empty() {
        return Err(ApiError(anyhow::anyhow!("an archive reason is required")));
    }
    Ok(Json(
        state
            .client
            .archive_page(ArchivePageRequest {
                page_id: request.page_id,
                expected_revision_id: request.expected_revision_id,
                reason: Some(reason.to_owned()),
            })
            .await?,
    ))
}

async fn restore_governance_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<GovernanceMutationRequest>,
) -> Result<Json<pcp_core::PageLifecycleTransitionResult>, ApiError> {
    require_console_mutation(&headers)?;
    let reason = request.reason.trim();
    if reason.is_empty() {
        return Err(ApiError(anyhow::anyhow!("a restore reason is required")));
    }
    Ok(Json(
        state
            .client
            .restore_archived_page(RestoreArchivedPageRequest {
                page_id: request.page_id,
                expected_revision_id: request.expected_revision_id,
                reason: Some(reason.to_owned()),
            })
            .await?,
    ))
}

async fn run_semantic_search(
    State(state): State<AppState>,
    Json(request): Json<QueryContextRequest>,
) -> Result<Json<pcp_core::QueryContextResponse>, ApiError> {
    Ok(Json(state.client.semantic_search(request).await?))
}

async fn run_match_intent(
    State(state): State<AppState>,
    Json(request): Json<IntentQueryRequest>,
) -> Result<Json<pcp_core::QueryContextResponse>, ApiError> {
    let effort = request.intent_effort;
    Ok(Json(
        state
            .client
            .match_intent(
                QueryContextRequest {
                    query: request.query,
                    scopes: request.scopes,
                    result_limit: request.result_limit,
                    context_budget_chars: request.context_budget_chars,
                },
                effort,
            )
            .await?,
    ))
}

async fn console_page_result(
    client: &RemotePcpClient,
    result: ContentLibraryResult,
) -> Result<ConsolePageResult> {
    let ContentLibraryResult {
        hits,
        next_cursor,
        total_pages,
        total_content_chars,
    } = result;
    let capabilities = client.capabilities();
    let chunk_size = usize::try_from(capabilities.max_read_pages)
        .unwrap_or(1)
        .max(1);
    let page_ids = hits
        .iter()
        .map(|hit| hit.page_id.clone())
        .collect::<Vec<_>>();
    let mut metadata = HashMap::with_capacity(page_ids.len());
    for chunk in page_ids.chunks(chunk_size) {
        let pages = client
            .read_pages(ReadPagesRequest {
                page_ids: chunk.to_vec(),
                revision_ids: Vec::new(),
                projections: vec![Projection::Payload, Projection::Relations],
                max_chars: capabilities.max_read_chars,
            })
            .await?;
        for page in pages {
            metadata.insert(
                page.page.page_id.clone(),
                PageListMetadata {
                    preview_payload: page.revision.payload,
                    relation_stats: PageListRelationStats::from_relations(
                        &page.page.page_id,
                        &page.relations,
                    ),
                    source_span: page.revision.source_span,
                },
            );
        }
    }

    Ok(ConsolePageResult {
        hits: hits
            .into_iter()
            .map(|hit| {
                let page_metadata = metadata.remove(&hit.page_id);
                let source_span = page_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.source_span.clone());
                ConsolePageHit {
                    preview_payload: page_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.preview_payload.clone()),
                    relation_stats: page_metadata.map(|metadata| metadata.relation_stats),
                    source_span,
                    hit,
                }
            })
            .collect(),
        next_cursor,
        total_pages,
        total_content_chars,
    })
}

async fn page_detail(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let max_chars = state.client.capabilities().max_read_chars;
    let page = read_one_page(
        state.client.as_ref(),
        page_id,
        vec![
            Projection::Manifest,
            Projection::Summary,
            Projection::Validity,
            Projection::Payload,
            Projection::Sources,
            Projection::Facets,
            Projection::Relations,
            Projection::History,
        ],
        max_chars,
    )
    .await?;
    Ok(Json(json!(page)))
}

async fn page_preview(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Json<ConsolePagePreview>, ApiError> {
    let page = read_one_page(
        state.client.as_ref(),
        page_id,
        page_preview_projections(),
        state.client.capabilities().max_read_chars,
    )
    .await?;
    Ok(Json(ConsolePagePreview {
        preview_payload: page.revision.payload,
    }))
}

async fn page_raw(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let page = read_one_page(
        state.client.as_ref(),
        page_id,
        vec![
            Projection::Manifest,
            Projection::Payload,
            Projection::Sources,
            Projection::Provenance,
        ],
        128_000,
    )
    .await?;
    Ok(Json(json!(page)))
}

async fn page_media(
    State(state): State<AppState>,
    Path((page_id, source_index)): Path<(String, usize)>,
) -> Result<Response, ApiError> {
    let page = read_one_page(
        state.client.as_ref(),
        page_id,
        vec![Projection::Manifest, Projection::Sources],
        1_000,
    )
    .await?;
    let source_ref = page
        .revision
        .source_refs
        .get(source_index)
        .cloned()
        .context("PCP media SourceRef was not found")?;
    let roots = state.local_media_roots.clone();
    let (bytes, media_type, digest) =
        tokio::task::spawn_blocking(move || read_local_media(&source_ref, roots.as_slice()))
            .await
            .context("join PCP local media read")??;

    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&media_type).context("encode PCP media content type")?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"sha256-{digest}\"")).context("encode PCP media ETag")?,
    );
    Ok(response)
}

async fn page_graph(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
    Query(query): Query<GraphQuery>,
) -> Result<Json<Value>, ApiError> {
    let graph = graph_view::load_page_graph(
        state.client.as_ref(),
        page_id,
        selected_scopes(state.client.as_ref(), None),
        query.depth,
        query.limit,
    )
    .await?;
    Ok(Json(json!(graph)))
}

async fn page_lineage(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let root = read_one_page(
        state.client.as_ref(),
        page_id,
        vec![Projection::Manifest, Projection::History],
        8_000,
    )
    .await?;
    let total = root.history.len();
    let limit = usize::try_from(state.client.capabilities().max_read_pages)
        .unwrap_or(MAX_HISTORY_REVISIONS)
        .min(MAX_HISTORY_REVISIONS);
    let pages = state
        .client
        .read_pages(ReadPagesRequest {
            page_ids: Vec::new(),
            revision_ids: root.history.into_iter().take(limit).collect(),
            projections: vec![
                Projection::Manifest,
                Projection::Summary,
                Projection::Validity,
                Projection::Payload,
                Projection::Facets,
            ],
            max_chars: 96_000,
        })
        .await?;
    Ok(Json(json!({"pages": pages, "total": total})))
}

async fn health_metrics(
    State(state): State<AppState>,
    Query(query): Query<HealthQuery>,
) -> Result<Json<Value>, ApiError> {
    let snapshot = state
        .client
        .health_snapshot(
            selected_scopes(state.client.as_ref(), query.scope.as_deref()),
            query.hours.unwrap_or(24).clamp(1, 24 * 90),
        )
        .await?;
    Ok(Json(json!(snapshot)))
}

async fn query_audit(
    State(state): State<AppState>,
    Query(query): Query<HealthQuery>,
) -> Result<Json<Value>, ApiError> {
    let summary = state
        .client
        .query_audit_summary(
            selected_scopes(state.client.as_ref(), query.scope.as_deref()),
            query.hours.unwrap_or(24).clamp(1, 24 * 90),
        )
        .await?;
    Ok(Json(json!(summary)))
}

async fn retention_plan(
    State(state): State<AppState>,
    Query(query): Query<RetentionQuery>,
) -> Result<Json<Value>, ApiError> {
    let scopes = selected_scopes(state.client.as_ref(), query.scope.as_deref());
    let (plan, leases) = tokio::try_join!(
        state
            .client
            .plan_revision_retention(PlanRevisionRetentionRequest {
                scopes: scopes.clone(),
                policy: retention_policy(&query),
            }),
        state
            .client
            .active_revision_retention_leases(scopes, MAX_RETENTION_SAMPLE_LIMIT),
    )?;
    Ok(Json(json!({"plan": plan, "leases": leases})))
}

async fn maintenance_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let Some(config_path) = state.runtime_config.as_ref() else {
        return Ok(Json(json!({"available": false})));
    };
    let config = RuntimeConfig::load(config_path)?;
    let Some(maintenance) = config.maintenance else {
        return Ok(Json(json!({
            "available": false,
            "configPath": config_path,
        })));
    };
    let automation = RuntimeMaintainer::automation_status(&maintenance).await?;
    Ok(Json(json!({
        "available": true,
        "configurable": state.runtime.is_some(),
        "enabled": maintenance.enabled,
        "mode": maintenance.mode,
        "intervalSeconds": maintenance.interval_seconds,
        "maxIntervalSeconds": maintenance.max_interval_seconds,
        "maxJobsPerCycle": maintenance.max_jobs_per_cycle,
        "writeTrigger": {
            "minNewPages": maintenance.write_trigger.min_new_pages,
            "quietPeriodSeconds": maintenance.write_trigger.quiet_period_seconds,
            "maxWaitSeconds": maintenance.write_trigger.max_wait_seconds,
        },
        "automation": automation,
        "packing": {
            "enabled": maintenance.packing.enabled,
            "maxPages": maintenance.packing.max_pages,
            "maxInputChars": maintenance.packing.max_input_chars,
            "analysisWindowPages": maintenance.packing.effective_analysis_window_pages(),
            "routingCharsPerPage": maintenance.packing.routing_chars_per_page,
        },
    })))
}

async fn update_maintenance_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MaintenanceSettingsRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let runtime = state
        .runtime
        .as_ref()
        .context("PCP Console can update maintenance settings only in managed mode")?;
    let status = runtime
        .update_maintenance_settings(managed::MaintenanceSettings {
            enabled: request.enabled,
            mode: request.mode,
            min_new_pages: request.min_new_pages,
            quiet_period_seconds: request.quiet_period_seconds,
            max_wait_seconds: request.max_wait_seconds,
        })
        .await?;
    Ok(Json(json!({
        "saved": true,
        "restarted": true,
        "pid": status.pid,
    })))
}

async fn maintenance_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_request): Json<MaintenanceScanRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let config_path = state
        .runtime_config
        .as_ref()
        .context("PCP Console has no Runtime configuration path")?;
    let operator = MaintenanceOperator::load(config_path).await?;
    if operator.identity_id() != state.client.identity_id() {
        return Err(anyhow::anyhow!(
            "PCP maintenance configuration points to a different Store identity"
        )
        .into());
    }
    let scan = operator.scan_maintenance_work().await?;
    Ok(Json(json!(scan)))
}

async fn maintenance_archive_scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_request): Json<MaintenanceScanRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    Ok(Json(json!(operator.scan_archive_candidates().await?)))
}

async fn maintenance_archive_analyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeMaintenanceArchiveRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    Ok(Json(json!(operator.analyze_archive(request).await?)))
}

async fn maintenance_archive_apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<GovernanceMutationRequest>,
) -> Result<Json<pcp_core::PageLifecycleTransitionResult>, ApiError> {
    archive_page_for_governance(&state, &headers, request).await
}

async fn maintenance_analyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeMaintenancePacksRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let config_path = state
        .runtime_config
        .as_ref()
        .context("PCP Console has no Runtime configuration path")?;
    let operator = MaintenanceOperator::load(config_path).await?;
    if operator.identity_id() != state.client.identity_id() {
        return Err(anyhow::anyhow!(
            "PCP maintenance configuration points to a different Store identity"
        )
        .into());
    }
    let analysis = operator.analyze_packing(request).await?;
    Ok(Json(json!(analysis)))
}

async fn maintenance_converge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_request): Json<MaintenanceScanRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    let report = operator.converge_once().await?;
    let reviews = operator.pending_reviews();
    Ok(Json(json!({
        "report": report,
        "reviews": reviews,
        "settled": report.jobs_advanced == 0,
    })))
}

async fn maintenance_reviews(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let operator = maintenance_operator_for_console(&state).await?;
    Ok(Json(json!({"reviews": operator.pending_reviews()})))
}

async fn accept_maintenance_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(candidate_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    let item = operator
        .review_item(&candidate_id)
        .context("unknown PCP maintenance review candidate")?;
    let result = match item.payload {
        MaintenanceReviewPayload::Pack(candidate) => json!(
            operator
                .apply_pack(ApplyMaintenancePackRequest {
                    candidate_id: candidate.candidate_id,
                    pages: candidate
                        .pages
                        .into_iter()
                        .map(|page| PageRevisionRef {
                            page_id: page.page_id,
                            revision_id: page.revision_id,
                        })
                        .collect(),
                })
                .await?
        ),
        MaintenanceReviewPayload::Summary(candidate) => json!(
            operator
                .apply_summary(ApplyMaintenanceSummaryRequest {
                    candidate_id: candidate.candidate_id,
                    page_id: candidate.page_id,
                    revision_id: candidate.revision_id,
                    expected_summary_revision_id: candidate.expected_summary_revision_id,
                    content: candidate.content,
                })
                .await?
        ),
        MaintenanceReviewPayload::Relation(_candidate) if candidate_id.starts_with("mrr_") => {
            json!(operator.approve_relation_review(&candidate_id).await?)
        }
        MaintenanceReviewPayload::Relation(candidate) => {
            let result = operator
                .apply_relation(ApplyMaintenanceRelationRequest {
                    candidate_id: candidate.candidate_id,
                    pages: candidate.pages.map(|page| PageRevisionRef {
                        page_id: page.page_id,
                        revision_id: page.revision_id,
                    }),
                })
                .await?;
            operator
                .resolve_review(&candidate_id, MaintenanceReviewStatus::Accepted)
                .await?;
            return Ok(Json(json!({
                "candidateId": candidate_id,
                "status": "accepted",
                "result": result,
            })));
        }
        MaintenanceReviewPayload::Topic(candidate) => json!(
            operator
                .apply_topic(ApplyMaintenanceTopicRequest {
                    candidate_id: candidate.candidate_id,
                    title: candidate.title,
                    content: candidate.content,
                    pages: candidate
                        .pages
                        .into_iter()
                        .map(|page| PageRevisionRef {
                            page_id: page.page_id,
                            revision_id: page.revision_id,
                        })
                        .collect(),
                })
                .await?
        ),
        MaintenanceReviewPayload::Archive(candidate) => json!(
            state
                .client
                .archive_page(ArchivePageRequest {
                    page_id: candidate.page_id,
                    expected_revision_id: candidate.revision_id,
                    reason: Some(candidate.reason),
                })
                .await?
        ),
    };
    if !candidate_id.starts_with("mrr_") {
        operator
            .resolve_review(&candidate_id, MaintenanceReviewStatus::Accepted)
            .await?;
    }
    Ok(Json(json!({
        "candidateId": candidate_id,
        "status": "accepted",
        "result": result,
    })))
}

async fn reject_maintenance_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(candidate_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    let item = operator
        .review_item(&candidate_id)
        .context("unknown PCP maintenance review candidate")?;
    match item.payload {
        MaintenanceReviewPayload::Relation(_) if candidate_id.starts_with("mrr_") => {
            operator
                .reject_relation_review(&candidate_id, false)
                .await?;
        }
        MaintenanceReviewPayload::Relation(candidate) => {
            operator
                .reject_relation(ApplyMaintenanceRelationRequest {
                    candidate_id: candidate.candidate_id,
                    pages: candidate.pages.map(|page| PageRevisionRef {
                        page_id: page.page_id,
                        revision_id: page.revision_id,
                    }),
                })
                .await?;
            operator
                .resolve_review(&candidate_id, MaintenanceReviewStatus::Rejected)
                .await?;
        }
        _ => {
            operator
                .resolve_review(&candidate_id, MaintenanceReviewStatus::Rejected)
                .await?;
        }
    }
    Ok(Json(
        json!({"candidateId": candidate_id, "status": "rejected"}),
    ))
}

async fn defer_maintenance_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(candidate_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    operator
        .resolve_review(&candidate_id, MaintenanceReviewStatus::Deferred)
        .await?;
    Ok(Json(
        json!({"candidateId": candidate_id, "status": "snoozed", "snoozedForSeconds": 86400}),
    ))
}

async fn suppress_maintenance_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(candidate_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    let item = operator
        .review_item(&candidate_id)
        .context("unknown PCP maintenance review candidate")?;
    let MaintenanceReviewPayload::Relation(candidate) = item.payload else {
        return Err(anyhow::anyhow!("only relation reviews can be suppressed").into());
    };
    if candidate_id.starts_with("mrr_") {
        operator.reject_relation_review(&candidate_id, true).await?;
    } else {
        operator
            .suppress_relation(ApplyMaintenanceRelationRequest {
                candidate_id: candidate.candidate_id,
                pages: candidate.pages.map(|page| PageRevisionRef {
                    page_id: page.page_id,
                    revision_id: page.revision_id,
                }),
            })
            .await?;
        operator
            .resolve_review(&candidate_id, MaintenanceReviewStatus::Suppressed)
            .await?;
    }
    Ok(Json(
        json!({"candidateId": candidate_id, "status": "suppressed"}),
    ))
}

async fn maintenance_apply_pack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyMaintenancePackRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let config_path = state
        .runtime_config
        .as_ref()
        .context("PCP Console has no Runtime configuration path")?;
    let operator = MaintenanceOperator::load(config_path).await?;
    if operator.identity_id() != state.client.identity_id() {
        return Err(anyhow::anyhow!(
            "PCP maintenance configuration points to a different Store identity"
        )
        .into());
    }
    let result = operator.apply_pack(request).await?;
    Ok(Json(json!({"optimized": true, "result": result})))
}

async fn maintenance_repair_pack_split(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RepairPackSplitRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let unpacked = operator
        .unpack_page(UnpackPageRequest {
            page_id: request.page_id,
            expected_revision_id: request.expected_revision_id,
            idempotency_key: None,
        })
        .await?;
    let packs = pack_restored_parts(
        &state,
        &operator,
        unpacked.restored_pages.clone(),
        request.split_after_source_positions,
    )
    .await?;
    Ok(Json(json!({
        "unpacked": unpacked,
        "packs": packs,
        "repaired": true,
    })))
}

async fn maintenance_repack_restored(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RepackRestoredPackRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let packs = pack_restored_parts(
        &state,
        &operator,
        request.pages,
        request.split_after_source_positions,
    )
    .await?;
    Ok(Json(json!({"repacked": true, "packs": packs})))
}

async fn pack_restored_parts(
    state: &AppState,
    operator: &MaintenanceOperator,
    pages: Vec<PageRevisionRef>,
    mut split_after_source_positions: Vec<u64>,
) -> Result<Vec<pcp_core::WriteResult>, ApiError> {
    split_after_source_positions.sort_unstable();
    split_after_source_positions.dedup();
    if split_after_source_positions.is_empty() {
        return Err(anyhow::anyhow!("a repaired Pack needs at least one split boundary").into());
    }
    let mut restored = Vec::new();
    for batch in pages.chunks(20) {
        restored.extend(
            state
                .client
                .read_pages(ReadPagesRequest {
                    page_ids: batch.iter().map(|entry| entry.page_id.clone()).collect(),
                    revision_ids: Vec::new(),
                    projections: vec![Projection::Payload],
                    max_chars: 1,
                })
                .await?,
        );
    }
    let spans = restored
        .into_iter()
        .map(|page| {
            (
                page.page.page_id,
                (
                    page.revision.revision_id,
                    page.revision.source_span,
                    page.revision
                        .observed_at
                        .unwrap_or(page.revision.created_at),
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut groups = vec![Vec::new(); split_after_source_positions.len() + 1];
    let mut group_spans = vec![Vec::new(); split_after_source_positions.len() + 1];
    let mut group_times = vec![Vec::new(); split_after_source_positions.len() + 1];
    for entry in pages {
        let (current_revision_id, source_span, observed_at) = spans
            .get(&entry.page_id)
            .with_context(|| format!("restored Page {} is not readable", entry.page_id))?;
        if current_revision_id != &entry.revision_id {
            return Err(anyhow::anyhow!(
                "restored Page {} changed before repacking",
                entry.page_id
            )
            .into());
        }
        let span = source_span
            .as_ref()
            .with_context(|| format!("restored Page {} has no source span", entry.page_id))?;
        let mut group_index = split_after_source_positions.len();
        for (index, boundary) in split_after_source_positions.iter().enumerate() {
            if span.end <= *boundary {
                group_index = index;
                break;
            }
            if span.start <= *boundary {
                return Err(anyhow::anyhow!("split position cuts through a restored Page").into());
            }
        }
        groups[group_index].push(entry);
        group_spans[group_index].push(span.clone());
        group_times[group_index].push(observed_at.clone());
    }
    if groups.iter().any(|group| group.len() < 2) {
        return Err(anyhow::anyhow!("each repaired Pack group needs at least two Pages").into());
    }
    let mut packs = Vec::with_capacity(groups.len());
    for pages in groups {
        packs.push(
            operator
                .pack_pages(PackPagesRequest {
                    pages,
                    idempotency_key: None,
                })
                .await?,
        );
    }
    for (spans, times) in group_spans.iter().zip(&group_times) {
        for pair in spans.windows(2) {
            if pair[0].stream_id != pair[1].stream_id
                || pair[0].end.checked_add(1) != Some(pair[1].start)
            {
                return Err(anyhow::anyhow!(
                    "a repaired Pack group must be contiguous in one source stream"
                )
                .into());
            }
        }
        let parsed_times = times
            .iter()
            .map(|time| {
                DateTime::parse_from_rfc3339(time)
                    .map(|time| time.with_timezone(&Utc))
                    .with_context(|| format!("decode restored Page timestamp {time}"))
            })
            .collect::<Result<Vec<_>>>()?;
        if parsed_times.windows(2).any(|pair| {
            let seconds = pair[1].signed_duration_since(pair[0]).num_seconds();
            !(0..=15 * 60).contains(&seconds)
        }) {
            return Err(anyhow::anyhow!(
                "a repaired Pack group exceeds the 15-minute continuity limit"
            )
            .into());
        }
    }
    Ok(packs)
}

async fn maintenance_retire_pack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RetirePackRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let current_revision_id = state.client.current_revision_id(request.page_id).await?;
    if current_revision_id != request.expected_revision_id {
        return Err(anyhow::anyhow!("packed Page changed before retirement").into());
    }
    let operator = maintenance_operator_for_console(&state).await?;
    let result = operator
        .retire_page(
            current_revision_id,
            Actor {
                actor_type: ActorType::Tool,
                actor_id: "tool:service:pcp-console-pack-repair".to_owned(),
            },
        )
        .await?;
    Ok(Json(json!({"retired": true, "result": result})))
}

async fn maintenance_analyze_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeMaintenanceSummaryRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let analysis = operator.analyze_summary(request).await?;
    Ok(Json(json!(analysis)))
}

async fn maintenance_analyze_summaries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeMaintenanceSummariesRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let analysis = operator.analyze_summaries(request).await?;
    Ok(Json(json!(analysis)))
}

async fn maintenance_apply_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyMaintenanceSummaryRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let result = operator.apply_summary(request).await?;
    Ok(Json(json!({"optimized": true, "result": result})))
}

async fn maintenance_analyze_relation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeMaintenanceRelationRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let analysis = operator.analyze_relation(request).await?;
    Ok(Json(json!(analysis)))
}

async fn maintenance_apply_relation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyMaintenanceRelationRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    let result = operator.apply_relation(request).await?;
    Ok(Json(json!({"optimized": true, "result": result})))
}

async fn maintenance_suppress_relation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyMaintenanceRelationRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    operator.suppress_relation(request).await?;
    Ok(Json(json!({"suppressed": true})))
}

async fn maintenance_reject_relation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyMaintenanceRelationRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    operator.reject_relation(request).await?;
    Ok(Json(json!({"rejected": true})))
}

async fn maintenance_analyze_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnalyzeMaintenanceTopicRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    Ok(Json(json!(operator.analyze_topic(request).await?)))
}

async fn maintenance_apply_topic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyMaintenanceTopicRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let operator = maintenance_operator_for_console(&state).await?;
    Ok(Json(
        json!({"optimized": true, "result": operator.apply_topic(request).await?}),
    ))
}

async fn maintenance_relation_reviews(
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let operator = maintenance_operator_for_console(&state).await?;
    Ok(Json(
        json!({"proposals": operator.pending_relation_reviews()}),
    ))
}

async fn approve_relation_review(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    let relation = operator.approve_relation_review(&candidate_id).await?;
    Ok(Json(json!({"approved": true, "relation": relation})))
}

async fn reject_relation_review(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<RelationReviewDecisionRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    let mut operator = maintenance_operator_for_console(&state).await?;
    operator
        .reject_relation_review(&candidate_id, request.suppress)
        .await?;
    Ok(Json(
        json!({"rejected": true, "suppressed": request.suppress}),
    ))
}

async fn maintenance_operator_for_console(
    state: &AppState,
) -> Result<MaintenanceOperator, ApiError> {
    let config_path = state
        .runtime_config
        .as_ref()
        .context("PCP Console has no Runtime configuration path")?;
    let operator = MaintenanceOperator::load(config_path).await?;
    if operator.identity_id() != state.client.identity_id() {
        return Err(anyhow::anyhow!(
            "PCP maintenance configuration points to a different Store identity"
        )
        .into());
    }
    Ok(operator)
}

async fn access_log(
    State(state): State<AppState>,
    Query(query): Query<AccessQuery>,
) -> Result<Json<Value>, ApiError> {
    let (events, next_cursor) = state
        .client
        .access_log(query.limit.unwrap_or(100).clamp(1, 500), query.cursor)
        .await?;
    Ok(Json(json!({"events": events, "nextCursor": next_cursor})))
}

async fn enrollment_snapshot(
    State(state): State<AppState>,
) -> Result<Json<EnrollmentAdminResponse>, ApiError> {
    Ok(Json(state.enrollment.snapshot().await?))
}

async fn approve_enrollment(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<EnrollmentAdminResponse>, ApiError> {
    require_console_mutation(&headers)?;
    Ok(Json(state.enrollment.approve(request_id).await?))
}

async fn reject_enrollment(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<EnrollmentAdminResponse>, ApiError> {
    require_console_mutation(&headers)?;
    Ok(Json(state.enrollment.reject(request_id).await?))
}

async fn revoke_enrollment(
    State(state): State<AppState>,
    Path(registration_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<EnrollmentAdminResponse>, ApiError> {
    require_console_mutation(&headers)?;
    Ok(Json(state.enrollment.revoke(registration_id).await?))
}

fn require_console_mutation(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers
        .get("x-pcp-console")
        .and_then(|value| value.to_str().ok())
        != Some("1")
    {
        return Err(anyhow::anyhow!("missing PCP Console mutation header").into());
    }
    Ok(())
}

async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install PCP Console SIGTERM handler");
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                eprintln!("PCP Console interrupt handler failed: {error}");
            }
        }
        _ = terminate.recv() => {}
    }
}

fn selected_scopes(client: &RemotePcpClient, selected: Option<&str>) -> Vec<String> {
    if let Some(scope) = selected.map(str::trim).filter(|scope| !scope.is_empty()) {
        return vec![scope.to_owned()];
    }
    let mut scopes = client
        .access()
        .grants
        .iter()
        .map(|grant| grant.namespace.clone())
        .collect::<Vec<_>>();
    scopes.sort();
    scopes.dedup();
    scopes
}

fn local_media_roots() -> Result<Vec<PathBuf>> {
    let Some(configured) = env::var_os(LOCAL_MEDIA_ROOTS_ENV) else {
        return Ok(Vec::new());
    };
    let mut roots = env::split_paths(&configured)
        .map(|path| {
            fs::canonicalize(&path)
                .with_context(|| format!("resolve PCP Console local media root {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn read_local_media(
    source_ref: &SourceRef,
    allowed_roots: &[PathBuf],
) -> Result<(Vec<u8>, String, String)> {
    anyhow::ensure!(
        !allowed_roots.is_empty(),
        "PCP Console local media preview is disabled; configure {LOCAL_MEDIA_ROOTS_ENV}"
    );
    let media_type = source_ref
        .media_type
        .as_deref()
        .context("PCP local media SourceRef has no mediaType")?;
    anyhow::ensure!(
        matches!(
            media_type,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/avif"
        ),
        "PCP Console does not render this local media type"
    );
    let expected_digest = source_ref
        .content_digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .context("PCP local media SourceRef requires a sha256 contentDigest")?;
    anyhow::ensure!(
        expected_digest.len() == 64 && expected_digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "PCP local media SourceRef has an invalid sha256 contentDigest"
    );
    let locator: LocalSourceLocator =
        serde_json::from_str(&source_ref.locator).context("decode PCP local media locator")?;
    let url = Url::parse(&locator.uri).context("parse PCP local media URI")?;
    anyhow::ensure!(
        url.scheme() == "file",
        "PCP Console does not fetch remote media"
    );
    let original_path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("PCP local media URI is not a valid file path"))?;
    let link_metadata = fs::symlink_metadata(&original_path)
        .with_context(|| format!("inspect PCP local media {}", original_path.display()))?;
    anyhow::ensure!(
        !link_metadata.file_type().is_symlink(),
        "PCP local media file cannot be a symlink"
    );
    let canonical_path = fs::canonicalize(&original_path)
        .with_context(|| format!("resolve PCP local media {}", original_path.display()))?;
    anyhow::ensure!(
        allowed_roots
            .iter()
            .any(|root| canonical_path.starts_with(root)),
        "PCP local media file is outside configured roots"
    );

    let mut file = File::open(&canonical_path)
        .with_context(|| format!("open PCP local media {}", canonical_path.display()))?;
    let metadata = file.metadata().context("read PCP local media metadata")?;
    anyhow::ensure!(
        metadata.is_file(),
        "PCP local media source is not a regular file"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_LOCAL_MEDIA_BYTES,
        "PCP local media exceeds the preview size limit"
    );
    ensure_current_user_owned(&metadata)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .context("read PCP local media bytes")?;
    anyhow::ensure!(
        bytes.len() as u64 == metadata.len(),
        "PCP local media changed while being read"
    );
    let actual_digest = format!("{:x}", Sha256::digest(&bytes));
    anyhow::ensure!(
        actual_digest.eq_ignore_ascii_case(expected_digest),
        "PCP local media contentDigest does not match"
    );
    Ok((bytes, media_type.to_owned(), actual_digest))
}

#[cfg(unix)]
fn ensure_current_user_owned(metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    anyhow::ensure!(
        metadata.uid() == unsafe { libc::geteuid() },
        "PCP local media file is not owned by the Console user"
    );
    Ok(())
}

#[cfg(not(unix))]
fn ensure_current_user_owned(_metadata: &fs::Metadata) -> Result<()> {
    anyhow::bail!("PCP local media preview requires owner-aware filesystem metadata")
}

async fn read_one_page(
    client: &RemotePcpClient,
    page_id: String,
    projections: Vec<Projection>,
    max_chars: u32,
) -> Result<ReadPage, ApiError> {
    let mut pages = client
        .read_pages(ReadPagesRequest {
            page_ids: (!page_id.starts_with("rev_"))
                .then_some(page_id.clone())
                .into_iter()
                .collect(),
            revision_ids: page_id
                .starts_with("rev_")
                .then_some(page_id)
                .into_iter()
                .collect(),
            projections,
            max_chars,
        })
        .await?;
    pages
        .pop()
        .ok_or_else(|| anyhow::anyhow!("PCP Page was not found").into())
}

fn page_preview_projections() -> Vec<Projection> {
    vec![Projection::Payload]
}

fn parse_browse_index_order(value: Option<&str>) -> Result<BrowseIndexOrder, ApiError> {
    match value.unwrap_or("recent") {
        "recent" => Ok(BrowseIndexOrder::Recent),
        "oldest" => Ok(BrowseIndexOrder::Oldest),
        "most_connected" => Ok(BrowseIndexOrder::MostConnected),
        "least_connected" => Ok(BrowseIndexOrder::LeastConnected),
        "largest" => Ok(BrowseIndexOrder::Largest),
        "source_order" => Ok(BrowseIndexOrder::SourceOrder),
        other => Err(anyhow::anyhow!("unsupported console page order: {other}").into()),
    }
}

fn retention_policy(query: &RetentionQuery) -> RetentionPolicy {
    let defaults = RetentionPolicy::default();
    RetentionPolicy {
        minimum_age_days: query
            .minimum_age_days
            .unwrap_or(defaults.minimum_age_days)
            .min(36_500),
        keep_recent_revisions_per_page: query
            .keep_recent_revisions_per_page
            .unwrap_or(defaults.keep_recent_revisions_per_page)
            .min(1_000),
        sample_limit: query
            .sample_limit
            .unwrap_or(defaults.sample_limit)
            .clamp(1, MAX_RETENTION_SAMPLE_LIMIT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_static_assets_disable_browser_caching() {
        let response = static_asset("text/plain; charset=utf-8", "console");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            CONSOLE_STATIC_CACHE_CONTROL
        );
    }

    #[test]
    fn console_page_preview_reads_only_the_payload_projection() {
        assert!(matches!(
            page_preview_projections().as_slice(),
            [Projection::Payload]
        ));
    }

    #[test]
    fn console_page_preview_uses_the_browser_wire_field_name() {
        let preview = ConsolePagePreview {
            preview_payload: Some(PagePayload {
                media_type: "text/plain".to_owned(),
                content: "recovered preview".to_owned(),
            }),
        };

        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            json!({
                "previewPayload": {
                    "mediaType": "text/plain",
                    "content": "recovered preview"
                }
            })
        );
    }

    fn local_source_ref(path: &std::path::Path, digest: &str) -> SourceRef {
        SourceRef {
            provider_id: "test-local-media".to_owned(),
            locator: json!({
                "uri": Url::from_file_path(path).expect("test file URL").to_string()
            })
            .to_string(),
            media_type: Some("image/png".to_owned()),
            content_digest: Some(format!("sha256:{digest}")),
        }
    }

    fn relation(relation_id: &str, from_page_id: &str, to_page_id: &str) -> Relation {
        Relation {
            relation_id: relation_id.to_owned(),
            from_page_id: from_page_id.to_owned(),
            relation_type: "related_to".to_owned(),
            to_page_id: to_page_id.to_owned(),
            basis_revision_ids: Vec::new(),
            created_by: pcp_core::Actor {
                actor_type: pcp_core::ActorType::System,
                actor_id: "console-test".to_owned(),
            },
            created_at: "2026-08-16T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn console_browse_orders_are_explicitly_bounded() {
        assert_eq!(
            parse_browse_index_order(None).unwrap(),
            BrowseIndexOrder::Recent
        );
        assert_eq!(
            parse_browse_index_order(Some("most_connected")).unwrap(),
            BrowseIndexOrder::MostConnected
        );
        assert_eq!(
            parse_browse_index_order(Some("source_order")).unwrap(),
            BrowseIndexOrder::SourceOrder
        );
        assert!(parse_browse_index_order(Some("relevance")).is_err());
    }

    #[test]
    fn console_retention_policy_defaults_and_bounds_samples() {
        let defaults = retention_policy(&RetentionQuery::default());
        assert_eq!(defaults.minimum_age_days, 30);
        assert_eq!(defaults.keep_recent_revisions_per_page, 2);
        assert_eq!(defaults.sample_limit, 100);

        let bounded = retention_policy(&RetentionQuery {
            minimum_age_days: Some(u32::MAX),
            keep_recent_revisions_per_page: Some(u32::MAX),
            sample_limit: Some(0),
            ..RetentionQuery::default()
        });
        assert_eq!(bounded.minimum_age_days, 36_500);
        assert_eq!(bounded.keep_recent_revisions_per_page, 1_000);
        assert_eq!(bounded.sample_limit, 1);
    }

    #[test]
    fn console_relation_stats_distinguish_incoming_and_outgoing_edges() {
        let relations = vec![
            relation("rel-in", "pg-source", "pg-current"),
            relation("rel-out", "pg-current", "pg-target"),
            relation("rel-other", "pg-left", "pg-right"),
        ];

        assert_eq!(
            PageListRelationStats::from_relations("pg-current", &relations),
            PageListRelationStats {
                total: 2,
                incoming: 1,
                outgoing: 1,
            }
        );
    }

    #[test]
    fn local_media_requires_allowed_path_and_matching_digest() {
        let unique = format!(
            "pcp-console-media-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock")
                .as_nanos()
        );
        let root = env::temp_dir().join(unique);
        let outside = root.join("outside");
        let allowed = root.join("allowed");
        fs::create_dir_all(&outside).expect("create outside test directory");
        fs::create_dir_all(&allowed).expect("create allowed test directory");
        let image = allowed.join("sample.png");
        let bytes = b"bounded image bytes";
        fs::write(&image, bytes).expect("write test image");
        let digest = format!("{:x}", Sha256::digest(bytes));
        let source_ref = local_source_ref(&image, &digest);
        let allowed = fs::canonicalize(&allowed).expect("canonical allowed root");

        let (loaded, media_type, loaded_digest) =
            read_local_media(&source_ref, std::slice::from_ref(&allowed))
                .expect("read allowed local media");
        assert_eq!(loaded, bytes);
        assert_eq!(media_type, "image/png");
        assert_eq!(loaded_digest, digest);

        let outside = fs::canonicalize(&outside).expect("canonical outside root");
        assert!(read_local_media(&source_ref, &[outside]).is_err());
        let mismatched = local_source_ref(&image, &"0".repeat(64));
        assert!(read_local_media(&mismatched, &[allowed]).is_err());
        fs::remove_dir_all(&root).expect("remove local media test directory");
    }
}
