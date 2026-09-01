//! Direct operator actions; editor requests cannot replace provenance or sources.
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use pcp_client::{PcpApi, PcpTenantApi};
use pcp_core::{
    AccessPermission, DeletePageRequest, LifecycleStatus, PagePayload, Projection, ReadPage,
    ReadPagesRequest, RepairPageRequest, WriteResult,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, AppState, require_console_mutation};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EditRequest {
    expected_revision_id: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteRequest {
    expected_revision_id: String,
}

fn editable_payload(page: &ReadPage) -> anyhow::Result<&PagePayload> {
    anyhow::ensure!(
        page.page.lifecycle_status != LifecycleStatus::Tombstoned,
        "Page is deleted"
    );
    let payload = page
        .revision
        .payload
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("This Page has no editable text"))?;
    let media_type = payload.media_type.split(';').next().unwrap_or("").trim();
    anyhow::ensure!(
        matches!(media_type, "text/plain" | "text/markdown"),
        "Only plain text and Markdown Pages can be edited here; unpack structured Packs first"
    );
    anyhow::ensure!(
        !payload
            .content
            .contains("[projection truncated by host budget]"),
        "Page is too large for safe editing in Console"
    );
    Ok(payload)
}

async fn read_editable(state: &AppState, page_id: String) -> anyhow::Result<ReadPage> {
    let page = state
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![page_id],
            revision_ids: Vec::new(),
            projections: vec![
                Projection::Manifest,
                Projection::Payload,
                Projection::Sources,
                Projection::Facets,
            ],
            max_chars: state.client.capabilities().max_read_chars,
        })
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Page not found"))?;
    anyhow::ensure!(
        state
            .client
            .access()
            .allows(&page.page.namespace, AccessPermission::Repair),
        "Page editing is not authorized"
    );
    editable_payload(&page)?;
    Ok(page)
}

pub(crate) async fn edit_content(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let page = read_editable(&state, page_id).await?;
    Ok(Json(
        json!({"pageId":page.page.page_id, "revisionId":page.revision.revision_id, "content":editable_payload(&page)?.content}),
    ))
}

pub(crate) async fn save_content(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<EditRequest>,
) -> Result<Json<WriteResult>, ApiError> {
    require_console_mutation(&headers)?;
    let page = read_editable(&state, page_id.clone()).await?;
    if page.revision.revision_id != request.expected_revision_id {
        return Err(anyhow::anyhow!(
            "revision conflict: Page changed while editing; your draft has not been saved"
        )
        .into());
    }
    let old_payload = editable_payload(&page)?;
    if old_payload.content == request.content {
        return Ok(Json(WriteResult {
            page_id,
            revision_id: page.revision.revision_id,
            created: false,
        }));
    }
    let result = state
        .client
        .repair_page(RepairPageRequest {
            page_id,
            expected_revision_id: request.expected_revision_id,
            payload: Some(PagePayload {
                media_type: old_payload.media_type.clone(),
                content: request.content,
            }),
            source_refs: page.revision.source_refs,
            facets: page.revision.facets,
            based_on_revision_ids: Vec::new(),
            reason: "Edited in PCP Console".to_owned(),
            tool_or_model: Some("pcp-console".to_owned()),
            idempotency_key: None,
        })
        .await?;
    Ok(Json(result))
}

pub(crate) async fn delete_page(
    State(state): State<AppState>,
    Path(page_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<DeleteRequest>,
) -> Result<Json<WriteResult>, ApiError> {
    require_console_mutation(&headers)?;
    Ok(Json(
        state
            .client
            .delete_page(DeletePageRequest {
                page_id,
                expected_revision_id: request.expected_revision_id,
                reason: Some("Deleted in PCP Console".to_owned()),
                idempotency_key: None,
            })
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_cannot_override_sources_or_actor() {
        assert!(
            serde_json::from_value::<EditRequest>(
                json!({"expectedRevisionId":"rev_1", "content":"fixed", "sourceRefs":[]})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<DeleteRequest>(
                json!({"expectedRevisionId":"rev_1", "actor":"operator"})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<EditRequest>(json!({"content":"fixed"})).is_err());
    }
}
