use anyhow::{Context, Result};
use pcp_core::{
    Actor, ArchivePageRequest, LifecycleStatus, PageLifecycleTransitionResult,
    RestoreArchivedPageRequest,
};
use rusqlite::params;

use crate::{
    SqlitePcpStore,
    write::{now, random_id},
};

impl SqlitePcpStore {
    pub async fn archive_page(
        &self,
        request: ArchivePageRequest,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<PageLifecycleTransitionResult> {
        self.transition_page_lifecycle(
            request.page_id,
            request.expected_revision_id,
            request.reason,
            actor,
            allowed_scopes,
            LifecycleStatus::Active,
            LifecycleStatus::Archived,
            "archive",
        )
        .await
    }

    pub async fn restore_archived_page(
        &self,
        request: RestoreArchivedPageRequest,
        actor: Actor,
        allowed_scopes: Vec<String>,
    ) -> Result<PageLifecycleTransitionResult> {
        self.transition_page_lifecycle(
            request.page_id,
            request.expected_revision_id,
            request.reason,
            actor,
            allowed_scopes,
            LifecycleStatus::Archived,
            LifecycleStatus::Active,
            "restore_archive",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn transition_page_lifecycle(
        &self,
        page_id: String,
        expected_revision_id: String,
        reason: Option<String>,
        actor: Actor,
        allowed_scopes: Vec<String>,
        expected_status: LifecycleStatus,
        next_status: LifecycleStatus,
        operation: &'static str,
    ) -> Result<PageLifecycleTransitionResult> {
        let reason = normalize_reason(reason)?;
        self.run("content governance lifecycle transition", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP lifecycle transition")?;
            let (namespace, current_revision_id, page_status, revision_status):
                (String, String, String, String) = transaction
                .query_row(
                    "
                    SELECT page.namespace, page.current_revision_id,
                           page.lifecycle_status, revision.lifecycle_status
                    FROM pcp_pages page
                    JOIN pcp_revisions revision
                      ON revision.revision_id = page.current_revision_id
                    WHERE page.page_id = ?1
                    ",
                    [&page_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .with_context(|| format!("find PCP Page {page_id} for content governance"))?;
            anyhow::ensure!(
                allowed_scopes.contains(&namespace),
                "page is outside the authorized PCP scopes"
            );
            anyhow::ensure!(
                current_revision_id == expected_revision_id,
                "revision conflict: expected {expected_revision_id}, current revision is {current_revision_id}"
            );
            anyhow::ensure!(
                page_status == expected_status.as_str() && revision_status == expected_status.as_str(),
                "PCP Page lifecycle conflict: expected {}, found page={} revision={}",
                expected_status.as_str(),
                page_status,
                revision_status,
            );

            let changed_at = now();
            transaction
                .execute(
                    "
                    UPDATE pcp_pages
                    SET lifecycle_status = ?2, updated_at = ?3
                    WHERE page_id = ?1 AND current_revision_id = ?4
                    ",
                    params![page_id, next_status.as_str(), changed_at, expected_revision_id],
                )
                .context("update PCP Page lifecycle")?;
            transaction
                .execute(
                    "
                    UPDATE pcp_revisions
                    SET lifecycle_status = ?2
                    WHERE revision_id = ?1 AND page_id = ?3
                    ",
                    params![expected_revision_id, next_status.as_str(), page_id],
                )
                .context("update current PCP Revision lifecycle")?;
            transaction
                .execute(
                    "
                    INSERT INTO pcp_page_lifecycle_events (
                        event_id, page_id, revision_id, previous_status, next_status,
                        actor_type, actor_id, reason, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    ",
                    params![
                        random_id(&transaction, "lifecycle_")?,
                        page_id,
                        expected_revision_id,
                        expected_status.as_str(),
                        next_status.as_str(),
                        actor.actor_type.as_str(),
                        actor.actor_id,
                        reason,
                        changed_at,
                    ],
                )
                .context("record PCP lifecycle transition")?;
            transaction
                .commit()
                .context("commit PCP lifecycle transition")?;
            Ok(PageLifecycleTransitionResult {
                page_id,
                revision_id: expected_revision_id,
                previous_lifecycle_status: expected_status,
                lifecycle_status: next_status,
                operation: operation.to_owned(),
                changed_at,
            })
        })
        .await
    }
}

fn normalize_reason(reason: Option<String>) -> Result<Option<String>> {
    let reason = reason.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    });
    if let Some(value) = reason.as_deref() {
        anyhow::ensure!(
            value.chars().count() <= 1_200,
            "content governance reason exceeds 1200 characters"
        );
    }
    Ok(reason)
}
