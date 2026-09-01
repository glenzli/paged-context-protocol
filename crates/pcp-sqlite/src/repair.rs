use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{Actor, PACKED_PAGE_MEDIA_TYPE, RepairPageRequest, SourceSpan, WriteResult};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::{
    SqlitePcpStore,
    write::{
        complete_provenance, ensure_provenance_access, insert_revision, lookup_write_idempotency,
        now, random_id, record_idempotency, validate_document,
    },
};

const MAX_REPAIR_REASON_CHARS: usize = 2_000;
const MAX_REPAIR_TOOL_CHARS: usize = 240;

impl SqlitePcpStore {
    /// Replace the current content of a Page through the administrative repair
    /// path while preserving the immutable previous Revision.
    pub(crate) async fn repair_page(
        &self,
        request: RepairPageRequest,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        validate_repair_request(&request)?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("Page repair", move |mut connection| {
            let transaction = connection.transaction().context("start PCP Page repair")?;
            let current = current_repair_state(&transaction, &request.page_id)?;
            anyhow::ensure!(
                allowed_scopes.contains(&current.namespace),
                "Page is outside the authorized PCP scopes"
            );
            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &actor.actor_id,
                "repair_page",
                request.idempotency_key.as_deref(),
            )? {
                anyhow::ensure!(
                    existing.page_id == request.page_id,
                    "Page repair idempotency key was already used for another Page"
                );
                return Ok(existing);
            }

            anyhow::ensure!(
                current.revision_id == request.expected_revision_id,
                "revision conflict: expected {}, current revision is {}",
                request.expected_revision_id,
                current.revision_id
            );
            anyhow::ensure!(
                current.media_type.as_deref() != Some(PACKED_PAGE_MEDIA_TYPE),
                "packed PCP Pages cannot be repaired; unpack them first"
            );
            anyhow::ensure!(
                current.lifecycle_status != "tombstoned",
                "tombstoned PCP Pages cannot be repaired"
            );
            let summary_target: Option<String> = transaction.query_row(
                "SELECT target_revision_id FROM pcp_summaries WHERE summary_revision_id = ?1",
                [&current.revision_id], |row| row.get(0),
            ).optional()?;
            if summary_target.is_some() {
                let content = request.payload.as_ref().context("Summary repair requires text")?;
                anyhow::ensure!(content.media_type == "text/markdown" && (1..=crate::summary::MAX_SUMMARY_CHARS).contains(&content.content.trim().chars().count()), "Summary repair requires 1-1200 Markdown characters");
            }

            let timestamp = now();
            let revision_id = random_id(&transaction, "rev_")?;
            let mut provenance_inputs = request.based_on_revision_ids.clone();
            provenance_inputs.push(request.expected_revision_id.clone());
            let mut provenance =
                complete_provenance(Vec::new(), "repair", &actor, &timestamp, provenance_inputs)?;
            let repair_event = provenance
                .first_mut()
                .context("construct PCP repair provenance")?;
            repair_event.tool_or_model = request
                .tool_or_model
                .as_deref()
                .map(str::trim)
                .map(str::to_owned);
            repair_event.reason = Some(request.reason.trim().to_owned());
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;

            insert_revision(
                &transaction,
                &request.page_id,
                &revision_id,
                &current.namespace,
                &current.lifecycle_status,
                &timestamp,
                current.observed_at.as_deref(),
                current.source_span.as_ref(),
                current.valid_from.as_deref(),
                current.valid_to.as_deref(),
                &actor,
                request.payload.as_ref(),
                &request.source_refs,
                request.facets.as_ref(),
                &provenance,
            )?;
            let published = transaction
                .execute(
                    "UPDATE pcp_pages
                     SET current_revision_id = ?2, updated_at = ?4
                     WHERE page_id = ?1 AND current_revision_id = ?3",
                    params![
                        request.page_id,
                        revision_id,
                        request.expected_revision_id,
                        timestamp,
                    ],
                )
                .context("publish repaired PCP Page revision")?;
            anyhow::ensure!(
                published == 1,
                "revision conflict while publishing Page repair"
            );
            // A Summary is a Page as well as a routing projection. Keep its
            // target binding and search projection in the same transaction.
            if let Some(target) = summary_target {
                transaction.execute("INSERT INTO pcp_summaries (summary_revision_id, target_revision_id) VALUES (?1, ?2)", params![revision_id, target])?;
                transaction.execute("DELETE FROM pcp_summary_fts WHERE summary_revision_id IN (SELECT revision_id FROM pcp_revisions WHERE page_id = ?1)", [&request.page_id])?;
                transaction.execute("INSERT INTO pcp_summary_fts (summary_revision_id, target_revision_id, content) VALUES (?1, ?2, ?3)", params![revision_id, target, request.payload.as_ref().unwrap().content])?;
            }
            record_idempotency(
                &transaction,
                &actor.actor_id,
                "repair_page",
                request.idempotency_key.as_deref(),
                Some(&request.page_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction.commit().context("commit PCP Page repair")?;
            Ok(WriteResult {
                page_id: request.page_id,
                revision_id,
                created: true,
            })
        })
        .await
    }
}

struct CurrentRepairState {
    namespace: String,
    lifecycle_status: String,
    revision_id: String,
    media_type: Option<String>,
    observed_at: Option<String>,
    source_span: Option<SourceSpan>,
    valid_from: Option<String>,
    valid_to: Option<String>,
}

fn current_repair_state(
    transaction: &Transaction<'_>,
    page_id: &str,
) -> Result<CurrentRepairState> {
    let state = transaction
        .query_row(
            "SELECT page.namespace, page.lifecycle_status, page.current_revision_id,
                    revision.payload_media_type, revision.observed_at,
                    revision.source_span_json, revision.valid_from, revision.valid_to
             FROM pcp_pages page
             JOIN pcp_revisions revision
               ON revision.revision_id = page.current_revision_id
             WHERE page.page_id = ?1",
            [page_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .with_context(|| format!("find current PCP Page revision for repair {page_id}"))?;
    let source_span = state
        .5
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .context("decode current PCP repair source span")?;
    Ok(CurrentRepairState {
        namespace: state.0,
        lifecycle_status: state.1,
        revision_id: state.2,
        media_type: state.3,
        observed_at: state.4,
        source_span,
        valid_from: state.6,
        valid_to: state.7,
    })
}

fn validate_repair_request(request: &RepairPageRequest) -> Result<()> {
    let reason_chars = request.reason.trim().chars().count();
    anyhow::ensure!(
        (1..=MAX_REPAIR_REASON_CHARS).contains(&reason_chars),
        "Page repair reason must contain 1-{MAX_REPAIR_REASON_CHARS} characters"
    );
    if let Some(tool_or_model) = request.tool_or_model.as_deref() {
        let chars = tool_or_model.trim().chars().count();
        anyhow::ensure!(
            (1..=MAX_REPAIR_TOOL_CHARS).contains(&chars),
            "Page repair toolOrModel must contain 1-{MAX_REPAIR_TOOL_CHARS} characters"
        );
    }
    validate_document(request.payload.as_ref(), &request.source_refs)?;
    Ok(())
}
