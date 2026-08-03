use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{ConsolidatePagesRequest, WriteResult};
use rusqlite::params;

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
        request
            .replaced_pages
            .sort_by(|left, right| left.page_id.cmp(&right.page_id));
        request
            .replaced_pages
            .dedup_by(|left, right| left.page_id == right.page_id);
        anyhow::ensure!(
            (1..MAX_CONSOLIDATION_INPUTS).contains(&request.replaced_pages.len()),
            "PCP consolidation requires 1-{} absorbed current Pages",
            MAX_CONSOLIDATION_INPUTS - 1
        );
        anyhow::ensure!(
            !request
                .replaced_pages
                .iter()
                .any(|input| input.page_id == request.canonical_page_id),
            "the canonical Page must not be repeated in replacedPages"
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

            let (owner_id, namespace, visibility, current_revision_id): (
                String,
                String,
                String,
                String,
            ) = transaction
                .query_row(
                    "
                    SELECT owner_id, namespace, visibility, current_revision_id
                    FROM pcp_pages
                    WHERE page_id = ?1
                    ",
                    [&request.canonical_page_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .context("find canonical PCP Page")?;
            anyhow::ensure!(
                current_revision_id == request.expected_canonical_revision_id,
                "canonical Page changed before consolidation"
            );
            anyhow::ensure!(
                allowed_scopes.contains(&namespace),
                "canonical Page is outside the authorized PCP scopes"
            );

            let mut replaced_pages = Vec::with_capacity(request.replaced_pages.len());
            for input in &request.replaced_pages {
                ensure_revision_access(&transaction, &input.expected_revision_id, &allowed_scopes)?;
                let (target_owner, target_namespace, target_visibility, target_head): (
                    String,
                    String,
                    String,
                    String,
                ) = transaction
                    .query_row(
                        "
                        SELECT owner_id, namespace, visibility, current_revision_id
                        FROM pcp_pages
                        WHERE page_id = ?1 AND lifecycle_status = 'active'
                        ",
                        [&input.page_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .with_context(|| {
                        format!("find active consolidation input {}", input.page_id)
                    })?;
                anyhow::ensure!(
                    target_owner == owner_id
                        && target_namespace == namespace
                        && target_visibility == visibility,
                    "PCP consolidation inputs must share owner, Scope, and visibility"
                );
                anyhow::ensure!(
                    target_head == input.expected_revision_id,
                    "consolidation input {} changed before commit",
                    input.page_id
                );
                replaced_pages.push((input.page_id.clone(), input.expected_revision_id.clone()));
                let already_replaced: bool = transaction
                    .query_row(
                        "
                        SELECT EXISTS (
                            SELECT 1
                            FROM pcp_relations relation
                            WHERE relation.relation_type = 'supersedes'
                              AND relation.to_page_id = ?1
                              AND NOT EXISTS (
                                  SELECT 1 FROM pcp_relation_retractions retraction
                                  WHERE retraction.relation_id = relation.relation_id
                              )
                        )
                        ",
                        [&input.page_id],
                        |row| row.get(0),
                    )
                    .context("check consolidation input standing")?;
                anyhow::ensure!(
                    !already_replaced,
                    "consolidation input {} has already been superseded",
                    input.page_id
                );
            }

            let timestamp = now();
            let mut provenance_inputs = request
                .replaced_pages
                .iter()
                .map(|input| input.expected_revision_id.clone())
                .collect::<Vec<_>>();
            provenance_inputs.push(request.expected_canonical_revision_id.clone());
            let provenance = complete_provenance(
                request.provenance,
                "consolidate",
                &request.created_by,
                &timestamp,
                provenance_inputs,
            )?;
            ensure_provenance_access(&transaction, &provenance, &allowed_scopes)?;

            let revision_id = random_id(&transaction, "rev_")?;
            insert_revision(
                &transaction,
                &request.canonical_page_id,
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
            for (_, replaced_revision_id) in &replaced_pages {
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
                    params![
                        request.canonical_page_id,
                        revision_id,
                        request.expected_canonical_revision_id
                    ],
                )
                .context("publish consolidated PCP Page")?;
            anyhow::ensure!(
                published == 1,
                "canonical PCP Page changed during consolidation"
            );
            for (replaced_page_id, expected_revision_id) in replaced_pages {
                let retired = transaction
                    .execute(
                        "
                        UPDATE pcp_pages
                        SET lifecycle_status = 'superseded', updated_at = ?2
                        WHERE page_id = ?1 AND current_revision_id = ?3
                        ",
                        params![replaced_page_id, timestamp, expected_revision_id],
                    )
                    .context("retire replaced PCP Page")?;
                anyhow::ensure!(
                    retired == 1,
                    "replaced PCP Page changed during consolidation"
                );
            }
            record_idempotency(
                &transaction,
                &request.created_by.actor_id,
                "consolidate_pages",
                request.idempotency_key.as_deref(),
                Some(&request.canonical_page_id),
                Some(&revision_id),
                None,
                &timestamp,
            )?;
            transaction
                .commit()
                .context("commit PCP Page consolidation")?;
            Ok(WriteResult {
                page_id: request.canonical_page_id,
                revision_id,
                created: true,
            })
        })
        .await
    }
}
