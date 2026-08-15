use std::{env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use pcp_client::{PcpApi, PcpTenantApi};
use pcp_core::{
    PlanRevisionRetentionRequest, Projection, ReadPage, ReadPagesRequest, RetentionPolicy,
    SearchFilters, SearchMode, SearchPagesRequest, SearchTermMatch,
};
use pcp_rpc::{EnrollmentAdminClient, EnrollmentAdminResponse, RemotePcpClient};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{net::TcpListener, time::Instant};

const DEFAULT_BIND: &str = "127.0.0.1:4318";
const DEFAULT_PRINCIPAL_ID: &str = "operator:local";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_PAGE_LIMIT: u32 = 20;
const MAX_PAGE_LIMIT: u32 = 50;
const MAX_HISTORY_REVISIONS: usize = 20;
const MAX_RETENTION_SAMPLE_LIMIT: u32 = 100;

mod graph_view;
mod managed;

#[derive(Clone)]
struct AppState {
    client: Arc<RemotePcpClient>,
    enrollment: EnrollmentAdminClient,
    runtime: Option<Arc<managed::ManagedRuntime>>,
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("{:#}", self.0)})),
        )
            .into_response()
    }
}

#[derive(Default, Deserialize)]
struct PageQuery {
    q: Option<String>,
    scope: Option<String>,
    mode: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Default, Deserialize)]
struct AccessQuery {
    cursor: Option<String>,
    limit: Option<u32>,
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
        .route("/quality-view.js", get(quality_view_js))
        .route("/health-view.js", get(health_view_js))
        .route("/retention-view.js", get(retention_view_js))
        .route("/styles.css", get(styles_css))
        .route("/api/health", get(health))
        .route("/api/runtime", get(runtime_status))
        .route("/api/runtime/restart", post(restart_runtime))
        .route("/api/overview", get(overview))
        .route("/api/pages", get(pages))
        .route("/api/pages/{page_id}", get(page_detail))
        .route("/api/pages/{page_id}/raw", get(page_raw))
        .route("/api/pages/{page_id}/graph", get(page_graph))
        .route("/api/pages/{page_id}/lineage", get(page_lineage))
        .route("/api/quality", get(quality))
        .route("/api/metrics", get(health_metrics))
        .route("/api/retention", get(retention_plan))
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

async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("app.js"),
    )
}

async fn page_inspector_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("page-inspector.js"),
    )
}

async fn page_content_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("page-content.js"),
    )
}

async fn page_content_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("page-content.css"),
    )
}

async fn page_graph_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("page-graph.js"),
    )
}

async fn quality_view_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("quality-view.js"),
    )
}

async fn health_view_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("health-view.js"),
    )
}

async fn retention_view_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("retention-view.js"),
    )
}

async fn styles_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("styles.css"),
    )
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
    let (integrity, scopes, page_count, content_chars) = tokio::try_join!(
        state.client.integrity_check(),
        state.client.list_scopes(Vec::new(), None, 10_000, None),
        state.client.page_count(Vec::new()),
        state.client.content_char_count(Vec::new()),
    )?;
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
        "pageCount": page_count,
        "contentChars": content_chars,
        "scopes": scopes.0,
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
    let result = match query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(text) => {
            state
                .client
                .search_pages(SearchPagesRequest {
                    query: text.to_owned(),
                    scopes,
                    mode: parse_search_mode(query.mode.as_deref())?,
                    term_match: SearchTermMatch::All,
                    projections: vec![
                        Projection::Manifest,
                        Projection::Summary,
                        Projection::Validity,
                        Projection::Facets,
                    ],
                    filters: SearchFilters::default(),
                    limit,
                    cursor: query.cursor,
                })
                .await?
        }
        None => {
            state
                .client
                .browse_index(scopes, Vec::new(), limit, query.cursor, 40_000)
                .await?
        }
    };
    Ok(Json(json!(result)))
}

async fn page_detail(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let page = read_one_page(
        state.client.as_ref(),
        page_id,
        vec![
            Projection::Manifest,
            Projection::Summary,
            Projection::Validity,
            Projection::Payload,
            Projection::Facets,
            Projection::Relations,
            Projection::History,
        ],
        8_000,
    )
    .await?;
    Ok(Json(json!(page)))
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

async fn quality(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let items = state.client.durable_page_inventory(Vec::new()).await?;
    Ok(Json(json!({"items": items})))
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

fn parse_search_mode(value: Option<&str>) -> Result<SearchMode, ApiError> {
    match value.unwrap_or("auto") {
        "auto" => Ok(SearchMode::Auto),
        "text" => Ok(SearchMode::Text),
        "exact" => Ok(SearchMode::Exact),
        other => Err(anyhow::anyhow!("unsupported console search mode: {other}").into()),
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
    fn console_search_modes_are_explicitly_bounded() {
        assert_eq!(parse_search_mode(None).unwrap(), SearchMode::Auto);
        assert_eq!(parse_search_mode(Some("text")).unwrap(), SearchMode::Text);
        assert_eq!(parse_search_mode(Some("exact")).unwrap(), SearchMode::Exact);
        assert!(parse_search_mode(Some("graph")).is_err());
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
}
