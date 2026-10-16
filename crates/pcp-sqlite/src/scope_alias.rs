//! Retired duplicate Scopes keep historical Revisions, but cannot receive new content.
use crate::{SqlitePcpStore, write::now};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn ensure_scope_writable(connection: &Connection, namespace: &str) -> Result<()> {
    let target: Option<String> = connection
        .query_row(
            "SELECT canonical_namespace FROM pcp_scope_aliases WHERE namespace = ?1",
            [namespace],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(target) = target {
        anyhow::bail!(
            "Scope {namespace} is a retired project alias; resolve its projectKey and write to {target}"
        );
    }
    Ok(())
}

impl SqlitePcpStore {
    pub async fn verify_scope_transfer_backup(&self, backup: &std::path::Path) -> Result<()> {
        anyhow::ensure!(
            backup.is_absolute()
                && std::fs::canonicalize(backup)? != std::fs::canonicalize(&self.path)?,
            "use a separate absolute Store backup"
        );
        let connection =
            Connection::open_with_flags(backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let identity: String = connection.query_row(
            "SELECT value FROM pcp_metadata WHERE key = 'identity_id'",
            [],
            |row| row.get(0),
        )?;
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        anyhow::ensure!(
            identity == self.identity_id() && integrity == "ok",
            "Store backup identity or integrity mismatch"
        );
        Ok(())
    }

    /// Operator-only retirement after all current Pages have been transferred.
    /// Original Revisions and every reference to them remain untouched.
    pub async fn retire_project_scope(&self, source: String, target: String) -> Result<bool> {
        anyhow::ensure!(
            source.starts_with("project:") && target.starts_with("project:") && source != target,
            "retirement requires two distinct project Scopes"
        );
        self.run("retire duplicate project Scope", move |mut connection| {
            let transaction = connection.transaction().context("start Scope retirement")?;
            if let Some(existing) = transaction.query_row("SELECT canonical_namespace FROM pcp_scope_aliases WHERE namespace = ?1", [&source], |row| row.get::<_,String>(0)).optional()? {
                anyhow::ensure!(existing == target, "project alias already points elsewhere");
                return Ok(false);
            }
            ensure_scope_writable(&transaction, &target)?;
            let pages: i64 = transaction.query_row("SELECT COUNT(*) FROM pcp_pages WHERE namespace = ?1", [&source], |row| row.get(0))?;
            anyhow::ensure!(pages == 0, "transfer all current Pages before retiring a project Scope");
            let children: i64 = transaction.query_row("SELECT COUNT(*) FROM pcp_scopes WHERE parent_namespace = ?1", [&source], |row| row.get(0))?;
            anyhow::ensure!(children == 0, "review child Scopes before retirement");
            transaction.execute("INSERT INTO pcp_scope_aliases(namespace,canonical_namespace,created_at) VALUES (?1,?2,?3)", params![source,target,now()])?;
            transaction.commit()?;
            Ok(true)
        }).await
    }
}
