use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use pcp_core::{Capabilities, Projection, SearchMode};
use rusqlite::Connection;
use tokio::task;

use crate::{
    audit_writer::{AccessAuditPolicy, AccessAuditWriter},
    schema,
};

pub const MAX_SEARCH_RESULTS: u32 = 50;
pub const MAX_READ_PAGES: u32 = 20;
pub const MAX_READ_CHARS: u32 = 64_000;
pub(crate) const MAX_PAGE_CHARS: usize = 256_000;

#[derive(Clone)]
pub struct SqlitePcpStore {
    pub(crate) path: PathBuf,
    identity_id: String,
    pub(crate) audit_writer: Arc<AccessAuditWriter>,
}

impl SqlitePcpStore {
    pub async fn open(path: PathBuf) -> Result<Self> {
        Self::open_with_access_audit_policy(path, AccessAuditPolicy::default()).await
    }

    pub(crate) async fn open_with_access_audit_policy(
        path: PathBuf,
        audit_policy: AccessAuditPolicy,
    ) -> Result<Self> {
        let path_for_open = path.clone();
        let (identity_id, audit_writer) = task::spawn_blocking(move || -> Result<_> {
            if let Some(parent) = path_for_open.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("create PCP database directory {}", parent.display())
                })?;
            }
            let mut connection = open_connection(&path_for_open)?;
            schema::initialize(&mut connection)?;
            let identity_id = schema::identity_id(&connection)?;
            drop(connection);
            let audit_writer = AccessAuditWriter::start(path_for_open, audit_policy)?;
            Ok((identity_id, Arc::new(audit_writer)))
        })
        .await
        .context("join PCP database initialization")??;
        Ok(Self {
            path,
            identity_id,
            audit_writer,
        })
    }

    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            protocol_version: "0.8.0-draft".to_owned(),
            search_modes: vec![
                SearchMode::Auto,
                SearchMode::Exact,
                SearchMode::Text,
                SearchMode::Graph,
                SearchMode::Temporal,
            ],
            projections: vec![
                Projection::Manifest,
                Projection::Summary,
                Projection::Validity,
                Projection::Payload,
                Projection::Sources,
                Projection::Provenance,
                Projection::Relations,
                Projection::Facets,
                Projection::History,
            ],
            max_search_results: MAX_SEARCH_RESULTS,
            max_read_pages: MAX_READ_PAGES,
            max_read_chars: MAX_READ_CHARS,
            features: [
                "access_audit",
                "lossless_page_packing",
                "revision_retention",
                "revision_retention_leases",
                "revision_retention_planning",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    pub(crate) async fn run<T, F>(&self, operation: &'static str, function: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Connection) -> Result<T> + Send + 'static,
    {
        let path = self.path.clone();
        task::spawn_blocking(move || {
            let connection = open_connection(&path)?;
            function(connection)
        })
        .await
        .with_context(|| format!("join PCP {operation}"))?
    }
}

pub(crate) fn open_connection(path: &Path) -> Result<Connection> {
    let connection =
        Connection::open(path).with_context(|| format!("open PCP database {}", path.display()))?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("enable PCP foreign keys")?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .context("configure PCP SQLite busy timeout")?;
    Ok(connection)
}
