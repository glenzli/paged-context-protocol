use std::collections::HashSet;

use crate::{SqlitePcpStore, audit_writer::AccessAuditRecord};
use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use pcp_core::{
    AccessAuditEvent, AccessDecision, AccessPermission, AccessSession, OperationTelemetry,
};

const MAX_AUDIT_DETAIL_CHARS: usize = 320;

pub(crate) async fn authorize_scopes(
    store: &SqlitePcpStore,
    access: &AccessSession,
    permissions: &[AccessPermission],
    requested_scopes: &[String],
) -> Result<Vec<String>> {
    if requested_scopes.is_empty() {
        let mut available = access.scopes_with_permissions(permissions);
        let store_wide = access.has_store_permissions(permissions);
        if store_wide {
            available.extend(store.local_scope_names().await?);
        }
        available.sort();
        available.dedup();
        if available.is_empty() && !store_wide {
            anyhow::bail!("PCP access session has no authorized Scope for this operation");
        }
        return Ok(available);
    }
    let mut resolved = Vec::with_capacity(requested_scopes.len());
    for scope in requested_scopes {
        if !permissions
            .iter()
            .all(|permission| access.allows(scope, *permission))
        {
            anyhow::bail!("PCP Scope is not authorized for this operation: {scope}");
        }
        if !resolved.contains(scope) {
            resolved.push(scope.clone());
        }
    }
    Ok(resolved)
}

pub(crate) async fn authorize_scopes_any(
    store: &SqlitePcpStore,
    access: &AccessSession,
    permissions: &[AccessPermission],
    requested_scopes: &[String],
) -> Result<Vec<String>> {
    if requested_scopes.is_empty() {
        let mut available = access
            .grants
            .iter()
            .filter(|grant| {
                permissions
                    .iter()
                    .any(|permission| grant.allows(*permission))
            })
            .map(|grant| grant.namespace.clone())
            .collect::<Vec<_>>();
        let store_wide = permissions
            .iter()
            .any(|permission| access.store_permissions.contains(permission));
        if store_wide {
            available.extend(store.local_scope_names().await?);
        }
        available.sort();
        available.dedup();
        if available.is_empty() && !store_wide {
            anyhow::bail!("PCP access session has no authorized Scope for this operation");
        }
        return Ok(available);
    }
    let mut resolved = Vec::with_capacity(requested_scopes.len());
    for scope in requested_scopes {
        if !permissions
            .iter()
            .any(|permission| access.allows(scope, *permission))
        {
            anyhow::bail!("PCP Scope is not authorized for this operation: {scope}");
        }
        if !resolved.contains(scope) {
            resolved.push(scope.clone());
        }
    }
    Ok(resolved)
}

pub(crate) fn authorize_exact(
    access: &AccessSession,
    namespace: &str,
    permission: AccessPermission,
) -> Result<()> {
    if !access.allows(namespace, permission) {
        anyhow::bail!(
            "PCP Scope is not authorized for {}: {namespace}",
            permission.as_str()
        );
    }
    Ok(())
}

impl SqlitePcpStore {
    pub(crate) async fn revision_namespaces(
        &self,
        revision_ids: Vec<String>,
    ) -> Result<Vec<String>> {
        if revision_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run("revision Scope lookup", move |connection| {
            let mut scopes = Vec::new();
            for revision_id in revision_ids {
                let namespace = connection
                    .query_row(
                        "SELECT namespace FROM pcp_revisions WHERE revision_id = ?1",
                        [&revision_id],
                        |row| row.get::<_, String>(0),
                    )
                    .with_context(|| format!("find PCP Revision {revision_id}"))?;
                if !scopes.contains(&namespace) {
                    scopes.push(namespace);
                }
            }
            Ok(scopes)
        })
        .await
    }

    pub(crate) async fn page_namespace(&self, page_id: String) -> Result<String> {
        self.run("page Scope lookup", move |connection| {
            connection
                .query_row(
                    "
                    SELECT revision.namespace
                    FROM pcp_pages page
                    JOIN pcp_revisions revision
                      ON revision.revision_id = page.current_revision_id
                    WHERE page.page_id = ?1
                    ",
                    [&page_id],
                    |row| row.get(0),
                )
                .with_context(|| format!("find PCP Page {page_id}"))
        })
        .await
    }

    pub(crate) async fn page_namespaces(&self, page_ids: Vec<String>) -> Result<Vec<String>> {
        if page_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run("Page Scope lookup", move |connection| {
            let mut scopes = Vec::new();
            for page_id in page_ids {
                let namespace = connection
                    .query_row(
                        "SELECT namespace FROM pcp_pages WHERE page_id = ?1",
                        [&page_id],
                        |row| row.get::<_, String>(0),
                    )
                    .with_context(|| format!("find PCP Page {page_id}"))?;
                if !scopes.contains(&namespace) {
                    scopes.push(namespace);
                }
            }
            Ok(scopes)
        })
        .await
    }

    pub(crate) async fn record_access(
        &self,
        access: &AccessSession,
        operation: &str,
        scopes: &[String],
        decision: &AccessDecision,
        detail: Option<&str>,
        telemetry: Option<&OperationTelemetry>,
    ) -> Result<()> {
        let occurred_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let principal_json = serde_json::to_string(&access.principal)?;
        let scopes_json = serde_json::to_string(scopes)?;
        let session_id = access.session_id.clone();
        let operation = operation.to_owned();
        let decision = decision.as_str().to_owned();
        let detail = detail.map(bound_detail);
        let telemetry_json = telemetry.map(serde_json::to_string).transpose()?;
        let durable = decision != AccessDecision::Allowed.as_str();
        let record = AccessAuditRecord {
            occurred_at,
            principal_json,
            session_id,
            operation,
            scopes_json,
            decision,
            detail,
            telemetry_json,
        };
        if durable {
            self.audit_writer.append_durable(record).await
        } else {
            self.audit_writer.enqueue(record).await
        }
    }

    pub(crate) async fn flush_access_audit(&self) -> Result<()> {
        self.audit_writer.flush().await
    }

    pub(crate) async fn read_access_log(
        &self,
        allowed_scopes: Vec<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<AccessAuditEvent>, Option<String>)> {
        self.flush_access_audit().await?;
        let allowed_scopes = allowed_scopes.into_iter().collect::<HashSet<_>>();
        let limit = limit.clamp(1, 100) as usize;
        let offset = cursor
            .as_deref()
            .unwrap_or("0")
            .parse::<usize>()
            .context("invalid PCP access log cursor")?;
        self.run("access audit read", move |connection| {
            let mut statement = connection
                .prepare(
                    "
                    SELECT event_id, occurred_at, principal_json, session_id,
                           operation, scopes_json, decision, detail, telemetry_json
                    FROM pcp_access_log
                    ORDER BY occurred_at DESC, event_id DESC
                    ",
                )
                .context("prepare PCP access log")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                })
                .context("query PCP access log")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect PCP access log")?;
            let mut events = rows
                .into_iter()
                .filter_map(
                    |(
                        event_id,
                        occurred_at,
                        principal_json,
                        session_id,
                        operation,
                        scopes_json,
                        decision,
                        detail,
                        telemetry_json,
                    )| {
                        let mut scopes = serde_json::from_str::<Vec<String>>(&scopes_json).ok()?;
                        scopes.retain(|scope| allowed_scopes.contains(scope));
                        if scopes.is_empty() {
                            return None;
                        }
                        Some(AccessAuditEvent {
                            event_id,
                            occurred_at,
                            principal: serde_json::from_str(&principal_json).ok()?,
                            session_id,
                            operation,
                            scopes,
                            decision: AccessDecision::parse(&decision)?,
                            detail,
                            telemetry: telemetry_json
                                .as_deref()
                                .and_then(|value| serde_json::from_str(value).ok()),
                        })
                    },
                )
                .skip(offset)
                .take(limit + 1)
                .collect::<Vec<_>>();
            let has_more = events.len() > limit;
            events.truncate(limit);
            Ok((events, has_more.then(|| (offset + limit).to_string())))
        })
        .await
    }
}

fn bound_detail(value: &str) -> String {
    if value.chars().count() <= MAX_AUDIT_DETAIL_CHARS {
        return value.to_owned();
    }
    value.chars().take(MAX_AUDIT_DETAIL_CHARS).collect()
}
