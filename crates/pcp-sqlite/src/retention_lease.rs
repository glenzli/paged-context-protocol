use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use pcp_core::{PutRevisionRetentionLeaseRequest, RevisionRetentionLease};
use rusqlite::{OptionalExtension, params, params_from_iter, types::Value as SqlValue};

use crate::{
    store::SqlitePcpStore,
    write::{now, random_id},
};

const MAX_REASON_CHARS: usize = 1_000;
const MAX_IDEMPOTENCY_KEY_CHARS: usize = 500;
const MAX_LEASES_PER_READ: u32 = 500;

impl SqlitePcpStore {
    pub async fn put_revision_retention_lease(
        &self,
        holder_principal_id: String,
        request: PutRevisionRetentionLeaseRequest,
    ) -> Result<RevisionRetentionLease> {
        validate_request(&holder_principal_id, &request)?;
        self.run("Revision retention lease write", move |mut connection| {
            let transaction = connection
                .transaction()
                .context("start PCP Revision retention lease write")?;
            let page_id = transaction
                .query_row(
                    "SELECT page_id FROM pcp_revisions
                     WHERE revision_id = ?1 AND namespace = ?2",
                    params![request.revision_id, request.namespace],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("find PCP Revision for retention lease")?
                .context("PCP Revision was not found in the requested Scope")?;
            let timestamp = now();
            let existing = transaction
                .query_row(
                    "SELECT lease_id, page_id, revision_id, namespace, created_at
                     FROM pcp_revision_retention_leases
                     WHERE holder_principal_id = ?1 AND idempotency_key = ?2",
                    params![holder_principal_id, request.idempotency_key],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    },
                )
                .optional()
                .context("read PCP Revision retention lease idempotency")?;
            let (lease_id, created_at) = if let Some((
                lease_id,
                existing_page_id,
                existing_revision_id,
                existing_namespace,
                created_at,
            )) = existing
            {
                anyhow::ensure!(
                    existing_page_id == page_id
                        && existing_revision_id == request.revision_id
                        && existing_namespace == request.namespace,
                    "PCP retention lease idempotency key was already used for another Revision"
                );
                transaction
                    .execute(
                        "UPDATE pcp_revision_retention_leases
                         SET reason = ?1, expires_at = ?2, updated_at = ?3
                         WHERE lease_id = ?4",
                        params![request.reason, request.expires_at, timestamp, lease_id],
                    )
                    .context("renew PCP Revision retention lease")?;
                (lease_id, created_at)
            } else {
                let lease_id = random_id(&transaction, "lease_")?;
                transaction
                    .execute(
                        "INSERT INTO pcp_revision_retention_leases (
                            lease_id, page_id, revision_id, namespace,
                            holder_principal_id, reason, created_at, updated_at,
                            expires_at, idempotency_key
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9)",
                        params![
                            lease_id,
                            page_id,
                            request.revision_id,
                            request.namespace,
                            holder_principal_id,
                            request.reason,
                            timestamp,
                            request.expires_at,
                            request.idempotency_key,
                        ],
                    )
                    .context("create PCP Revision retention lease")?;
                (lease_id, timestamp.clone())
            };
            transaction
                .commit()
                .context("commit PCP Revision retention lease")?;
            Ok(RevisionRetentionLease {
                lease_id,
                page_id,
                revision_id: request.revision_id,
                namespace: request.namespace,
                holder_principal_id,
                reason: request.reason,
                created_at,
                updated_at: timestamp,
                expires_at: request.expires_at,
            })
        })
        .await
    }

    pub async fn active_revision_retention_leases(
        &self,
        scopes: Vec<String>,
        limit: u32,
    ) -> Result<Vec<RevisionRetentionLease>> {
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, MAX_LEASES_PER_READ);
        self.run("Revision retention lease read", move |connection| {
            let mut sql = String::from(
                "SELECT lease_id, page_id, revision_id, namespace,
                        holder_principal_id, reason, created_at, updated_at, expires_at
                 FROM pcp_revision_retention_leases
                 WHERE expires_at > ?1 AND namespace IN (",
            );
            for index in 0..scopes.len() {
                if index > 0 {
                    sql.push(',');
                }
                sql.push('?');
                sql.push_str(&(index + 2).to_string());
            }
            sql.push_str(" ) ORDER BY expires_at ASC, lease_id ASC LIMIT ?");
            sql.push_str(&(scopes.len() + 2).to_string());
            let mut values = vec![SqlValue::Text(now())];
            values.extend(scopes.into_iter().map(SqlValue::Text));
            values.push(SqlValue::Integer(i64::from(limit)));
            let mut statement = connection
                .prepare(&sql)
                .context("prepare PCP active retention lease read")?;
            statement
                .query_map(params_from_iter(values.iter()), row_to_lease)
                .context("query PCP active retention leases")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect PCP active retention leases")
        })
        .await
    }
}

fn validate_request(
    holder_principal_id: &str,
    request: &PutRevisionRetentionLeaseRequest,
) -> Result<()> {
    anyhow::ensure!(
        !holder_principal_id.trim().is_empty(),
        "PCP retention lease holder must not be empty"
    );
    anyhow::ensure!(
        !request.namespace.trim().is_empty(),
        "PCP retention lease Scope must not be empty"
    );
    anyhow::ensure!(
        !request.revision_id.trim().is_empty(),
        "PCP retention lease Revision must not be empty"
    );
    let reason = request.reason.trim();
    anyhow::ensure!(
        !reason.is_empty() && reason.chars().count() <= MAX_REASON_CHARS,
        "PCP retention lease reason must contain 1-{MAX_REASON_CHARS} characters"
    );
    let idempotency_key = request.idempotency_key.trim();
    anyhow::ensure!(
        !idempotency_key.is_empty() && idempotency_key.chars().count() <= MAX_IDEMPOTENCY_KEY_CHARS,
        "PCP retention lease idempotency key must contain 1-{MAX_IDEMPOTENCY_KEY_CHARS} characters"
    );
    let expires_at = DateTime::parse_from_rfc3339(&request.expires_at)
        .context("parse PCP retention lease expiration")?
        .with_timezone(&Utc);
    anyhow::ensure!(
        expires_at > Utc::now(),
        "PCP retention lease expiration must be in the future"
    );
    Ok(())
}

fn row_to_lease(row: &rusqlite::Row<'_>) -> rusqlite::Result<RevisionRetentionLease> {
    Ok(RevisionRetentionLease {
        lease_id: row.get(0)?,
        page_id: row.get(1)?,
        revision_id: row.get(2)?,
        namespace: row.get(3)?,
        holder_principal_id: row.get(4)?,
        reason: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        expires_at: row.get(8)?,
    })
}
