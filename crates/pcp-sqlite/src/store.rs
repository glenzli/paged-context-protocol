use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pcp_core::{Capabilities, Projection, SearchMode};
use rusqlite::Connection;
use tokio::task;

use crate::schema;

pub const MAX_SEARCH_RESULTS: u32 = 50;
pub const MAX_READ_PAGES: u32 = 20;
pub const MAX_READ_CHARS: u32 = 64_000;
pub(crate) const MAX_PAGE_CHARS: usize = 256_000;

#[derive(Clone)]
pub struct SqlitePcpStore {
    pub(crate) path: PathBuf,
    owner_id: String,
}

impl SqlitePcpStore {
    pub async fn open(path: PathBuf) -> Result<Self> {
        let path_for_open = path.clone();
        let owner_id = task::spawn_blocking(move || -> Result<String> {
            if let Some(parent) = path_for_open.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("create PCP database directory {}", parent.display())
                })?;
            }
            let mut connection = open_connection(&path_for_open)?;
            schema::initialize(&mut connection)?;
            schema::owner_id(&connection)
        })
        .await
        .context("join PCP database initialization")??;
        Ok(Self { path, owner_id })
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            protocol_version: "0.5.0-draft".to_owned(),
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
            supports_event_ingest: true,
            supports_revision_conflicts: true,
            supports_durable_deletion: false,
            supports_provenance_graph: true,
            relation_types: vec![
                "contains",
                "aggregates",
                "derived_from",
                "summarizes",
                "follows",
                "responds_to",
                "continues",
                "has_attachment",
                "about",
                "updates",
                "depends_on",
                "defines",
                "uses",
                "supports",
                "contradicts",
                "supersedes",
                "qualifies",
                "reaffirms",
                "outdated_by",
                "inspired_by",
                "related_to",
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

fn open_connection(path: &Path) -> Result<Connection> {
    let connection =
        Connection::open(path).with_context(|| format!("open PCP database {}", path.display()))?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .context("enable PCP foreign keys")?;
    Ok(connection)
}
