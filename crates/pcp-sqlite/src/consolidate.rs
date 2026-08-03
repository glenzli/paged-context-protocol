use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{ConsolidatePagesRequest, WriteResult};
use rusqlite::{OptionalExtension, params};

use crate::{
    SqlitePcpStore,
    write::{
        complete_provenance, ensure_provenance_access, ensure_revision_access, insert_relation,
        insert_revision, lookup_write_idempotency, now, random_id, record_idempotency,
        validate_document,
    },
};

const MAX_CONSOLIDATION_INPUTS: usize = 64;

impl SqlitePcpStore {
    pub async fn consolidate_pages(
        &self,
        mut request: ConsolidatePagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<WriteResult> {
        validate_document(request.payload.as_ref(), &request.source_refs)?;
        request.replaced_revision_ids.sort();
        request.replaced_revision_ids.dedup();
        anyhow::ensure!(
            (2..=MAX_CONSOLIDATION_INPUTS).contains(&request.replaced_revision_ids.len()),
            "PCP consolidation requires 2-{MAX_CONSOLIDATION_INPUTS} distinct current Pages"
        );
        anyhow::ensure!(
            request
                .replaced_revision_ids
                .contains(&request.canonical_revision_id),
            "the canonical Page must be included in replacedPageIds"
        );

        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        self.run("page consolidation", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP Page consolidation")?;
            if let Some(existing) = lookup_write_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "consolidate_pages",
                request.idempotency_key.as_deref(),
            )? {
                return Ok(existing);
            }

            let (canonical_ref_id, owner_id, namespace, visibility, current_revision_id): (
                String,
                String,
                String,
                String,
                String,
            ) = transaction
                .query_row(
                    "
                    SELECT r.page_id, r.owner_id, r.namespace, r.visibility, ref.head_page_id
                    FROM pcp_revisions r
                    JOIN pcp_refs ref ON ref.ref_id = r.page_id
                    WHERE r.revision_id = ?1
                    ",
                    [&request.canonical_revision_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .context("find canonical PCP Page")?;
            anyhow::ensure!(
                current_revision_id == request.canonical_revision_id,
                "canonical Page is no longer the current head of its Ref"
            );
            anyhow::ensure!(
                allowed_scopes.contains(&namespace),
                "canonical Page is outside the authorized PCP scopes"
            );

            let mut replaced_refs = Vec::with_capacity(request.replaced_revision_ids.len());
            for revision_id in &request.replaced_revision_ids {
                ensure_revision_access(&transaction, revision_id, &allowed_scopes)?;
                let (target_owner, target_namespace, target_visibility, target_ref_id): (
                    String,
                    String,
                    String,
                    String,
                ) = transaction
                    .query_row(
                        "
                        SELECT owner_id, namespace, visibility, page_id
                        FROM pcp_revisions
                        WHERE revision_id = ?1 AND lifecycle_status = 'active'
                        ",
                        [revision_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .with_context(|| format!("find active consolidation input {revision_id}"))?;
                anyhow::ensure!(
                    target_owner == owner_id
                        && target_namespace == namespace
                        && target_visibility == visibility,
                    "PCP consolidation inputs must share owner, Scope, and visibility"
                );
                let target_head: Option<String> = transaction
                    .query_row(
                        "SELECT head_page_id FROM pcp_refs WHERE ref_id = ?1",
                        [&target_ref_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .context("read consolidation input Ref")?;
                anyhow::ensure!(
                    target_head.as_deref() == Some(revision_id.as_str()),
                    "consolidation input {revision_id} is no longer a current Ref head"
                );
                replaced_refs.push((target_ref_id, revision_id.clone()));
                let already_replaced: bool = transaction
                    .query_row(
                        "
                        SELECT EXISTS (
                            SELECT 1
                            FROM pcp_relations relation
                            WHERE relation.relation_type = 'supersedes'
                              AND relation.to_revision_id = ?1
                              AND NOT EXISTS (
                                  SELECT 1 FROM pcp_relation_retractions retraction
                                  WHERE retraction.relation_id = relation.relation_id
                              )
                        )
                        ",
                        [revision_id],
                        |row| row.get(0),
                    )
                    .context("check consolidation input standing")?;
                anyhow::ensure!(
                    !already_replaced,
                    "consolidation input {revision_id} has already been superseded"
                );
            }

            let timestamp = now();
            let provenance = complete_provenance(
                request.provenance,
                "consolidate",
                &request.created_by,
                &timestamp,
                request.replaced_revision_ids.clone(),
            )?;
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;

            let revision_id = random_id(&transaction, "rev_")?;
            insert_revision(
                &transaction,
                &canonical_ref_id,
                &revision_id,
                &owner_id,
                &namespace,
                &visibility,
                request.lifecycle_status.as_str(),
                &timestamp,
                request.observed_at.as_deref(),
                request.valid_from.as_deref(),
                request.valid_to.as_deref(),
                &request.created_by,
                request.payload.as_ref(),
                &request.source_refs,
                request.facets.as_ref(),
                &provenance,
            )?;
            for replaced_revision_id in &request.replaced_revision_ids {
                insert_relation(
                    &transaction,
                    &revision_id,
                    "supersedes",
                    replaced_revision_id,
                    &request.created_by,
                    &timestamp,
                )?;
            }

            let published = transaction
                .execute(
                    "
                    UPDATE pcp_pages
                    SET current_revision_id = ?2
                    WHERE page_id = ?1 AND current_revision_id = ?3
                    ",
                    params![canonical_ref_id, revision_id, request.canonical_revision_id],
                )
                .context("publish consolidated PCP Page")?;
            anyhow::ensure!(
                published == 1,
                "canonical PCP Page changed during consolidation"
            );
            let advanced = transaction
                .execute(
                    "
                    UPDATE pcp_refs
                    SET head_page_id = ?2, updated_at = ?3
                    WHERE ref_id = ?1 AND head_page_id = ?4
                    ",
                    params![
                        canonical_ref_id,
                        revision_id,
                        timestamp,
                        request.canonical_revision_id
                    ],
                )
                .context("advance canonical PCP Ref")?;
            anyhow::ensure!(
                advanced == 1,
                "canonical PCP Ref changed during consolidation"
            );
            for (replaced_ref_id, expected_revision_id) in replaced_refs {
                if replaced_ref_id == canonical_ref_id {
                    continue;
                }
                let redirected = transaction
                    .execute(
                        "
                        UPDATE pcp_refs
                        SET head_page_id = ?2, updated_at = ?3
                        WHERE ref_id = ?1 AND head_page_id = ?4
                        ",
                        params![
                            replaced_ref_id,
                            revision_id,
                            timestamp,
                            expected_revision_id
                        ],
                    )
                    .context("redirect replaced PCP Ref to consolidated Page")?;
                anyhow::ensure!(
                    redirected == 1,
                    "replaced PCP Ref changed during consolidation"
                );
            }
            record_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "consolidate_pages",
                request.idempotency_key.as_deref(),
                Some(&canonical_ref_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction
                .commit()
                .context("commit PCP Page consolidation")?;
            Ok(WriteResult {
                page_id: canonical_ref_id,
                revision_id,
                created: true,
            })
        })
        .await
    }
}
