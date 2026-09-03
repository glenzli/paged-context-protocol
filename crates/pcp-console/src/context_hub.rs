//! Operator UI bridge; tenant MCP never receives these management routes.
use crate::{ApiError, AppState, require_console_mutation};
use axum::{Json, extract::State, http::HeaderMap};
use pcp_client::{PcpTenantApi, context_hub::ContextHubRequest};
use serde_json::Value;

pub(crate) async fn inspect(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state.client.context_hub(ContextHubRequest::Inspect).await?,
    ))
}

pub(crate) async fn mutate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ContextHubRequest>,
) -> Result<Json<Value>, ApiError> {
    require_console_mutation(&headers)?;
    if !matches!(
        &request,
        ContextHubRequest::SetPolicy(_)
            | ContextHubRequest::Review(_)
            | ContextHubRequest::RemoveActivity { .. }
    ) {
        return Err(anyhow::anyhow!("unsupported Console context operation").into());
    }
    Ok(Json(state.client.context_hub(request).await?))
}
