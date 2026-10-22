//! Operator UI bridge; tenant MCP never receives these management routes.
use crate::{ApiError, AppState, require_console_mutation};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use pcp_client::{PcpTenantApi, context_hub::ContextHubRequest};
use serde_json::Value;

pub(crate) async fn inspect(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state.client.context_hub(ContextHubRequest::Inspect).await?,
    ))
}

pub(crate) async fn activity(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state
            .client
            .context_hub(ContextHubRequest::InspectActivity)
            .await?,
    ))
}

pub(crate) async fn policies(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state
            .client
            .context_hub(ContextHubRequest::InspectPolicies)
            .await?,
    ))
}

pub(crate) async fn inbox(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state
            .client
            .context_hub(ContextHubRequest::InspectInbox)
            .await?,
    ))
}

pub(crate) async fn candidate(
    State(state): State<AppState>,
    Path(candidate_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state
            .client
            .context_hub(ContextHubRequest::InspectCandidate { candidate_id })
            .await?,
    ))
}

pub(crate) async fn synthesis(
    State(state): State<AppState>,
    Path(synthesis_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        state
            .client
            .context_hub(ContextHubRequest::InspectSynthesis { synthesis_id })
            .await?,
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
            | ContextHubRequest::OrganizeCandidates
            | ContextHubRequest::ReviewSynthesis(_)
            | ContextHubRequest::StopSynthesis(_)
            | ContextHubRequest::SetAutomaticReview { .. }
            | ContextHubRequest::UndoAutomaticOutput { .. }
            | ContextHubRequest::RemoveActivity { .. }
    ) {
        return Err(anyhow::anyhow!("unsupported Console context operation").into());
    }
    Ok(Json(state.client.context_hub(request).await?))
}
