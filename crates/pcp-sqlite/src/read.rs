use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{Projection, ReadPage, ReadPagesRequest, Relation, Scope};
use rusqlite::params;

use crate::{
    row::{REVISION_COLUMNS, relation_from_row, revision_from_row},
    store::{MAX_READ_CHARS, MAX_READ_PAGES, SqlitePcpStore},
    summary::current_summary,
    validity::{current_validity, validity_history},
};

impl SqlitePcpStore {
    pub async fn local_scope_names(&self) -> Result<Vec<String>> {
        let owner_id = self.owner_id().to_owned();
        self.run("local scope discovery", move |connection| {
            let mut statement = connection
                .prepare(
                    "
                    SELECT namespace
                    FROM pcp_scopes
                    WHERE owner_id = ?1
                    ORDER BY namespace
                    ",
                )
                .context("prepare local PCP scopes")?;
            statement
                .query_map([owner_id], |row| row.get(0))
                .context("query local PCP scopes")?
                .collect::<rusqlite::Result<Vec<String>>>()
                .context("collect local PCP scopes")
        })
        .await
    }

    pub async fn integrity_check(&self) -> Result<String> {
        self.run("integrity check", move |connection| {
            connection
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .context("run PCP integrity check")
        })
        .await
    }

    pub async fn list_scopes(
        &self,
        allowed_scopes: Vec<String>,
        query: Option<String>,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<Scope>, Option<String>)> {
        let allowed_scopes: HashSet<String> = allowed_scopes.into_iter().collect();
        let query = query.unwrap_or_default().trim().to_lowercase();
        let limit = limit.clamp(1, 100) as usize;
        let offset = parse_cursor(cursor.as_deref())?;
        self.run("scope list", move |connection| {
            let mut statement = connection
                .prepare(
                    "
                    SELECT s.owner_id, s.namespace, s.scope_type, s.display_name,
                           s.description, s.parent_namespace, s.visibility,
                           s.created_at, s.updated_at, COUNT(p.page_id)
                    FROM pcp_scopes s
                    LEFT JOIN pcp_revisions r ON r.namespace = s.namespace
                    LEFT JOIN pcp_pages p ON p.current_revision_id = r.revision_id
                    GROUP BY s.namespace
                    ORDER BY s.updated_at DESC, s.namespace
                    ",
                )
                .context("prepare PCP scope list")?;
            let scopes = statement
                .query_map([], |row| {
                    Ok(Scope {
                        owner_id: row.get(0)?,
                        namespace: row.get(1)?,
                        scope_type: row.get(2)?,
                        display_name: row.get(3)?,
                        description: row.get(4)?,
                        parent_namespace: row.get(5)?,
                        visibility: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                        page_count: row.get::<_, i64>(9)? as u64,
                    })
                })
                .context("query PCP scopes")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect PCP scopes")?;
            let mut matching = scopes
                .into_iter()
                .filter(|scope| allowed_scopes.contains(&scope.namespace))
                .filter(|scope| {
                    query.is_empty()
                        || scope.namespace.to_lowercase().contains(&query)
                        || scope.display_name.to_lowercase().contains(&query)
                        || scope
                            .description
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&query)
                })
                .skip(offset)
                .take(limit + 1)
                .collect::<Vec<_>>();
            let has_more = matching.len() > limit;
            matching.truncate(limit);
            let next_cursor = has_more.then(|| (offset + limit).to_string());
            Ok((matching, next_cursor))
        })
        .await
    }

    pub async fn read_pages(
        &self,
        request: ReadPagesRequest,
        allowed_scopes: Vec<String>,
    ) -> Result<Vec<ReadPage>> {
        if request.revision_ids.is_empty() {
            return Ok(Vec::new());
        }
        if request.revision_ids.len() > MAX_READ_PAGES as usize {
            anyhow::bail!("read request exceeds the PCP page limit");
        }
        let allowed_scopes: HashSet<String> = allowed_scopes.into_iter().collect();
        let include_summary = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Summary);
        let include_payload = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Payload);
        let include_validity = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Validity);
        let include_facets = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Facets);
        let include_sources = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Sources);
        let include_provenance = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Provenance);
        let include_relations = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::Relations);
        let include_history = request
            .projections
            .iter()
            .any(|projection| projection == &Projection::History);
        let max_chars = request.max_chars.clamp(256, MAX_READ_CHARS) as usize;

        self.run("page read", move |connection| {
            let mut remaining_chars = max_chars;
            let mut output = Vec::with_capacity(request.revision_ids.len());
            for revision_id in request.revision_ids {
                let sql = format!(
                    "SELECT {REVISION_COLUMNS} FROM pcp_revisions r WHERE r.revision_id = ?1"
                );
                let mut statement = connection
                    .prepare(&sql)
                    .context("prepare PCP revision read")?;
                let mut rows = statement
                    .query([&revision_id])
                    .context("query PCP revision")?;
                let row = rows
                    .next()
                    .context("read PCP revision row")?
                    .context("PCP revision is not available")?;
                let mut revision = revision_from_row(
                    row,
                    include_payload,
                    include_facets,
                    include_sources,
                    include_provenance,
                )?;
                if !allowed_scopes.contains(&revision.namespace) {
                    anyhow::bail!("PCP revision is not available");
                }
                let mut summary = if include_summary {
                    current_summary(&connection, &revision_id)?
                } else {
                    None
                };
                let mut validity = if include_validity {
                    current_validity(&connection, &revision_id)?
                } else {
                    None
                };
                let mut validity_history = if include_validity && include_history {
                    validity_history(&connection, &revision_id)?
                } else {
                    Vec::new()
                };
                if let Some(validity) = validity.as_mut() {
                    bound_validity(validity, &mut remaining_chars);
                }
                for assessment in &mut validity_history {
                    bound_validity(assessment, &mut remaining_chars);
                }
                if let Some(summary) = summary.as_mut() {
                    let content_chars = summary.content.chars().count();
                    if content_chars > remaining_chars {
                        let retained = remaining_chars.saturating_sub(40);
                        summary.content = summary.content.chars().take(retained).collect();
                        summary
                            .content
                            .push_str("\n[projection truncated by host budget]");
                        remaining_chars = 0;
                    } else {
                        remaining_chars -= content_chars;
                    }
                }
                if let Some(payload) = revision.payload.as_mut() {
                    let content_chars = payload.content.chars().count();
                    if content_chars > remaining_chars {
                        let retained = remaining_chars.saturating_sub(40);
                        payload.content = payload.content.chars().take(retained).collect();
                        payload
                            .content
                            .push_str("\n[projection truncated by host budget]");
                        remaining_chars = 0;
                    } else {
                        remaining_chars -= content_chars;
                    }
                }
                let relations = if include_relations {
                    read_relations(&connection, &revision_id, &allowed_scopes)?
                } else {
                    Vec::new()
                };
                let history = if include_history {
                    let mut history_statement = connection
                        .prepare(
                            "
                            SELECT revision_id
                            FROM pcp_revisions
                            WHERE page_id = ?1
                            ORDER BY created_at DESC
                            LIMIT 100
                            ",
                        )
                        .context("prepare PCP revision history")?;
                    history_statement
                        .query_map([&revision.page_id], |row| row.get(0))
                        .context("query PCP revision history")?
                        .collect::<rusqlite::Result<Vec<String>>>()
                        .context("collect PCP revision history")?
                } else {
                    Vec::new()
                };
                output.push(ReadPage {
                    revision,
                    summary,
                    validity,
                    validity_history,
                    relations,
                    history,
                });
            }
            Ok(output)
        })
        .await
    }

    pub async fn current_revision_id(
        &self,
        page_id: String,
        allowed_scopes: Vec<String>,
    ) -> Result<String> {
        let allowed_scopes: HashSet<String> = allowed_scopes.into_iter().collect();
        self.run("current revision read", move |connection| {
            let (revision_id, namespace): (String, String) = connection
                .query_row(
                    "
                    SELECT p.current_revision_id, r.namespace
                    FROM pcp_pages p
                    JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
                    WHERE p.page_id = ?1
                    ",
                    [&page_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .context("find current PCP revision")?;
            if !allowed_scopes.contains(&namespace) {
                anyhow::bail!("PCP page is not available");
            }
            Ok(revision_id)
        })
        .await
    }

    pub async fn page_count(&self, allowed_scopes: Vec<String>) -> Result<u64> {
        let allowed_scopes: HashSet<String> = allowed_scopes.into_iter().collect();
        self.run("page count", move |connection| {
            let mut statement = connection
                .prepare(
                    "
                    SELECT r.namespace
                    FROM pcp_pages p
                    JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
                    ",
                )
                .context("prepare PCP page count")?;
            let count = statement
                .query_map([], |row| row.get::<_, String>(0))
                .context("query PCP page count")?
                .filter_map(Result::ok)
                .filter(|namespace| allowed_scopes.contains(namespace))
                .count();
            Ok(count as u64)
        })
        .await
    }

    pub async fn content_char_count(&self, allowed_scopes: Vec<String>) -> Result<usize> {
        let allowed_scopes: HashSet<String> = allowed_scopes.into_iter().collect();
        self.run("content size", move |connection| {
            let mut statement = connection
                .prepare(
                    "
                    SELECT r.namespace, length(COALESCE(r.payload_content, ''))
                    FROM pcp_pages p
                    JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
                    ",
                )
                .context("prepare PCP content size")?;
            let mut rows = statement.query([]).context("query PCP content size")?;
            let mut chars = 0_usize;
            while let Some(row) = rows.next().context("read PCP content size row")? {
                let namespace: String = row.get(0)?;
                if allowed_scopes.contains(&namespace) {
                    chars += row.get::<_, i64>(1)? as usize;
                }
            }
            Ok(chars)
        })
        .await
    }
}

fn bound_validity(validity: &mut pcp_core::PageValidity, remaining_chars: &mut usize) {
    validity.rationale = take_budgeted(&validity.rationale, remaining_chars);
    if let Some(scope) = validity.scope.as_mut() {
        *scope = take_budgeted(scope, remaining_chars);
    }
}

fn take_budgeted(content: &str, remaining_chars: &mut usize) -> String {
    let content_chars = content.chars().count();
    if content_chars <= *remaining_chars {
        *remaining_chars -= content_chars;
        return content.to_owned();
    }
    let retained = remaining_chars.saturating_sub(40);
    let mut output = content.chars().take(retained).collect::<String>();
    output.push_str("\n[projection truncated by host budget]");
    *remaining_chars = 0;
    output
}

fn read_relations(
    connection: &rusqlite::Connection,
    revision_id: &str,
    allowed_scopes: &HashSet<String>,
) -> Result<Vec<Relation>> {
    let mut statement = connection
        .prepare(
            "
            SELECT rel.relation_id, rel.from_revision_id, rel.relation_type,
                   rel.to_revision_id, rel.actor_type, rel.actor_id, rel.created_at,
                   source.namespace, target.namespace
            FROM pcp_relations rel
            JOIN pcp_revisions source ON source.revision_id = rel.from_revision_id
            JOIN pcp_revisions target ON target.revision_id = rel.to_revision_id
            WHERE rel.from_revision_id = ?1 OR rel.to_revision_id = ?1
            ORDER BY rel.created_at DESC
            LIMIT 200
            ",
        )
        .context("prepare PCP relations")?;
    let mut rows = statement
        .query(params![revision_id])
        .context("query PCP relations")?;
    let mut relations = Vec::new();
    while let Some(row) = rows.next().context("read PCP relation row")? {
        let source_namespace: String = row.get(7)?;
        let target_namespace: String = row.get(8)?;
        if allowed_scopes.contains(&source_namespace) && allowed_scopes.contains(&target_namespace)
        {
            relations.push(relation_from_row(row)?);
        }
    }
    Ok(relations)
}

fn parse_cursor(cursor: Option<&str>) -> Result<usize> {
    cursor
        .unwrap_or("0")
        .parse::<usize>()
        .context("invalid PCP pagination cursor")
}
