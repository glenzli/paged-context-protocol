use std::collections::HashSet;

use anyhow::{Context, Result};
use pcp_core::{
    BrowseIndexOrder, GraphSearchEdge, LifecycleStatus, PageValidity, PageValidityHint, Projection,
    SearchHit, SearchMode, SearchPagesRequest, SearchResult, SearchTermMatch,
};
use pcp_store::{ContentLibraryResult, ContentLibraryScope, ContentLibrarySummary};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};
use serde_json::{Map, Value};

use crate::{
    row::{REVISION_COLUMNS, page_manifest, revision_from_row},
    store::{MAX_SEARCH_RESULTS, SqlitePcpStore},
    text_ranking::{RelationPair, rank_text_hits},
    validity::current_validity,
};

impl SqlitePcpStore {
    pub async fn search_pages(&self, mut request: SearchPagesRequest) -> Result<SearchResult> {
        if request.scopes.is_empty() {
            anyhow::bail!("PCP search requires at least one authorized scope");
        }
        request.limit = request.limit.clamp(1, MAX_SEARCH_RESULTS);
        if request.projections.is_empty() {
            request.projections = pcp_core::default_search_projections();
        }
        let offset = parse_cursor(request.cursor.as_deref())?;
        self.run("page search", move |connection| {
            if request.mode == SearchMode::Auto {
                let primary_mode = if request.query.trim().is_empty() {
                    SearchMode::Temporal
                } else {
                    SearchMode::Text
                };
                let primary = search_once(
                    &connection,
                    &request,
                    primary_mode,
                    offset,
                    request.limit as usize,
                );
                if request.query.trim().is_empty() {
                    return primary;
                }
                if let Ok(result) = primary
                    && !result.hits.is_empty()
                {
                    return Ok(result);
                }
                return search_once(
                    &connection,
                    &request,
                    SearchMode::Exact,
                    offset,
                    request.limit as usize,
                );
            }
            search_once(
                &connection,
                &request,
                request.mode.clone(),
                offset,
                request.limit as usize,
            )
        })
        .await
    }

    pub async fn browse_index(
        &self,
        scopes: Vec<String>,
        excluded_page_kinds: Vec<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<SearchResult> {
        if scopes.is_empty() {
            anyhow::bail!("PCP index browse requires at least one authorized scope");
        }
        let limit = limit.clamp(1, crate::store::MAX_SEARCH_RESULTS) as usize;
        let offset = parse_cursor(cursor.as_deref())?;
        let max_chars = max_chars.clamp(1_000, 32_000) as usize;
        self.run("summary index browse", move |connection| {
            browse_index_once(
                &connection,
                &scopes,
                &excluded_page_kinds,
                &order,
                offset,
                limit,
                max_chars,
            )
        })
        .await
    }

    pub async fn browse_content_pages(
        &self,
        scopes: Vec<String>,
        query: Option<String>,
        order: BrowseIndexOrder,
        limit: u32,
        cursor: Option<String>,
        max_chars: u32,
    ) -> Result<ContentLibraryResult> {
        if scopes.is_empty() {
            anyhow::bail!("PCP content library browse requires at least one authorized scope");
        }
        let limit = limit.clamp(1, crate::store::MAX_SEARCH_RESULTS) as usize;
        let offset = parse_cursor(cursor.as_deref())?;
        let max_chars = max_chars.clamp(1_000, 32_000) as usize;
        let query = query.filter(|value| !value.trim().is_empty());
        self.run("content library browse", move |connection| {
            browse_content_pages_once(
                &connection,
                &scopes,
                query.as_deref(),
                &order,
                offset,
                limit,
                max_chars,
            )
        })
        .await
    }

    pub async fn content_library_summary(
        &self,
        scopes: Vec<String>,
    ) -> Result<ContentLibrarySummary> {
        if scopes.is_empty() {
            anyhow::bail!("PCP content library summary requires at least one authorized scope");
        }
        self.run("content library summary", move |connection| {
            content_library_summary_once(&connection, &scopes)
        })
        .await
    }
}

fn browse_index_once(
    connection: &Connection,
    scopes: &[String],
    excluded_page_kinds: &[String],
    order: &BrowseIndexOrder,
    offset: usize,
    limit: usize,
    max_chars: usize,
) -> Result<SearchResult> {
    let mut values = scopes
        .iter()
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    let mut sql = format!(
        "SELECT {REVISION_COLUMNS},
                substr(COALESCE(summary_revision.payload_content, r.payload_content, ''), 1, 700),
                CASE WHEN summary.summary_revision_id IS NULL THEN 'payload' ELSE 'summary' END,
                summary.summary_revision_id
         FROM pcp_pages p
         JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
         LEFT JOIN pcp_page_summary_heads summary_head
           ON summary_head.target_page_id = p.page_id
         LEFT JOIN pcp_pages summary_page
           ON summary_page.page_id = summary_head.summary_page_id
         LEFT JOIN pcp_summaries summary
           ON summary.summary_revision_id = summary_page.current_revision_id
         LEFT JOIN pcp_revisions summary_revision
           ON summary_revision.revision_id = summary.summary_revision_id
         LEFT JOIN (
           SELECT endpoint.page_id, COUNT(DISTINCT endpoint.relation_id) AS direct_relation_count
           FROM (
             SELECT relation.relation_id, relation.from_page_id AS page_id
             FROM pcp_relations relation
             WHERE NOT EXISTS (
               SELECT 1 FROM pcp_relation_retractions retraction
               WHERE retraction.relation_id = relation.relation_id
             )
             UNION ALL
             SELECT relation.relation_id, relation.to_page_id AS page_id
             FROM pcp_relations relation
             WHERE NOT EXISTS (
               SELECT 1 FROM pcp_relation_retractions retraction
               WHERE retraction.relation_id = relation.relation_id
             )
           ) endpoint
           GROUP BY endpoint.page_id
         ) relation_counts ON relation_counts.page_id = p.page_id
         WHERE r.namespace IN ("
    );
    push_placeholders(&mut sql, scopes.len());
    sql.push_str(
        ") AND r.lifecycle_status = 'active'
           AND NOT EXISTS (
               SELECT 1 FROM pcp_relations newer
               WHERE newer.relation_type = 'supersedes'
                 AND newer.to_page_id = p.page_id
                 AND NOT EXISTS (
                     SELECT 1 FROM pcp_relation_retractions retraction
                     WHERE retraction.relation_id = newer.relation_id
                 )
           )",
    );
    if !excluded_page_kinds.is_empty() {
        sql.push_str(" AND p.kind NOT IN (");
        push_placeholders(&mut sql, excluded_page_kinds.len());
        sql.push(')');
        values.extend(excluded_page_kinds.iter().cloned().map(SqlValue::Text));
    }
    sql.push_str(
        " AND (
             summary.summary_revision_id IS NOT NULL
             OR r.actor_type IN ('model', 'system')
         ) ORDER BY ",
    );
    sql.push_str(browse_index_order_by(order));
    sql.push_str(" LIMIT ? OFFSET ?");
    values.push(SqlValue::Integer((limit + 1) as i64));
    values.push(SqlValue::Integer(offset as i64));

    let mut statement = connection
        .prepare(&sql)
        .context("prepare PCP Summary index browse")?;
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .context("browse PCP Summary index")?;
    let mut hits = Vec::new();
    let mut used_chars = 0_usize;
    let mut has_more = false;
    while let Some(row) = rows.next().context("read PCP Summary index row")? {
        if hits.len() >= limit {
            has_more = true;
            break;
        }
        let revision = revision_from_row(row, false, true, false, false)?;
        let snippet: String = row.get(17)?;
        let matched_projection: String = row.get(18)?;
        let summary_revision_id: Option<String> = row.get(19)?;
        let entry_chars = snippet.chars().count().saturating_add(240);
        if !hits.is_empty() && used_chars.saturating_add(entry_chars) > max_chars {
            has_more = true;
            break;
        }
        used_chars = used_chars.saturating_add(entry_chars);
        let validity =
            current_validity(connection, &revision.revision_id)?.map(compact_validity_hint);
        let page = page_manifest(connection, &revision.page_id)?;
        hits.push(SearchHit {
            page_id: revision.page_id,
            revision_id: revision.revision_id,
            kind: page.kind,
            mutability: page.mutability,
            namespace: revision.namespace,
            lifecycle_status: revision.lifecycle_status,
            created_at: revision.created_at,
            observed_at: revision.observed_at,
            snippet,
            matched_by: "index_browse".to_owned(),
            matched_projection,
            summary_revision_id,
            facets: compact_search_facets(revision.facets),
            validity,
            graph_edges: Vec::new(),
        });
    }
    Ok(SearchResult {
        next_cursor: has_more.then(|| (offset + hits.len()).to_string()),
        hits,
    })
}

fn browse_content_pages_once(
    connection: &Connection,
    scopes: &[String],
    query: Option<&str>,
    order: &BrowseIndexOrder,
    offset: usize,
    limit: usize,
    max_chars: usize,
) -> Result<ContentLibraryResult> {
    let (total_pages, total_content_chars) =
        content_library_totals_once(connection, scopes, query)?;
    let mut values = scopes
        .iter()
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    let mut sql = format!(
        "SELECT {REVISION_COLUMNS},
                substr(COALESCE(r.payload_content, ''), 1, 700),
                'content',
                summary_page.current_revision_id
         FROM pcp_pages p
         JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
         LEFT JOIN pcp_page_summary_heads summary_head
           ON summary_head.target_page_id = p.page_id
         LEFT JOIN pcp_pages summary_page
           ON summary_page.page_id = summary_head.summary_page_id
         LEFT JOIN pcp_revisions summary_revision
           ON summary_revision.revision_id = summary_page.current_revision_id
         LEFT JOIN (
           SELECT endpoint.page_id, COUNT(DISTINCT endpoint.relation_id) AS direct_relation_count
           FROM (
             SELECT relation.relation_id, relation.from_page_id AS page_id
             FROM pcp_relations relation
             WHERE NOT EXISTS (
               SELECT 1 FROM pcp_relation_retractions retraction
               WHERE retraction.relation_id = relation.relation_id
             )
             UNION ALL
             SELECT relation.relation_id, relation.to_page_id AS page_id
             FROM pcp_relations relation
             WHERE NOT EXISTS (
               SELECT 1 FROM pcp_relation_retractions retraction
               WHERE retraction.relation_id = relation.relation_id
             )
           ) endpoint
           GROUP BY endpoint.page_id
         ) relation_counts ON relation_counts.page_id = p.page_id
         WHERE r.namespace IN ("
    );
    push_placeholders(&mut sql, scopes.len());
    sql.push(')');
    append_current_content_page_filter(&mut sql);
    append_content_library_query_filter(&mut sql, &mut values, query);
    sql.push_str(" ORDER BY ");
    sql.push_str(browse_index_order_by(order));
    sql.push_str(" LIMIT ? OFFSET ?");
    values.push(SqlValue::Integer((limit + 1) as i64));
    values.push(SqlValue::Integer(offset as i64));

    let mut statement = connection
        .prepare(&sql)
        .context("prepare PCP content library browse")?;
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .context("browse PCP content library")?;
    let mut hits = Vec::new();
    let mut used_chars = 0_usize;
    let mut has_more = false;
    while let Some(row) = rows.next().context("read PCP content library row")? {
        if hits.len() >= limit {
            has_more = true;
            break;
        }
        let revision = revision_from_row(row, false, true, false, false)?;
        let snippet: String = row.get(17)?;
        let matched_projection: String = row.get(18)?;
        let summary_revision_id: Option<String> = row.get(19)?;
        let entry_chars = snippet.chars().count().saturating_add(240);
        if !hits.is_empty() && used_chars.saturating_add(entry_chars) > max_chars {
            has_more = true;
            break;
        }
        used_chars = used_chars.saturating_add(entry_chars);
        let validity =
            current_validity(connection, &revision.revision_id)?.map(compact_validity_hint);
        let page = page_manifest(connection, &revision.page_id)?;
        hits.push(SearchHit {
            page_id: revision.page_id,
            revision_id: revision.revision_id,
            kind: page.kind,
            mutability: page.mutability,
            namespace: revision.namespace,
            lifecycle_status: revision.lifecycle_status,
            created_at: revision.created_at,
            observed_at: revision.observed_at,
            snippet,
            matched_by: if query.is_some() {
                "content_text".to_owned()
            } else {
                "content_library".to_owned()
            },
            matched_projection,
            summary_revision_id,
            facets: compact_search_facets(revision.facets),
            validity,
            graph_edges: Vec::new(),
        });
    }
    Ok(ContentLibraryResult {
        next_cursor: has_more.then(|| (offset + hits.len()).to_string()),
        hits,
        total_pages,
        total_content_chars,
    })
}

fn content_library_summary_once(
    connection: &Connection,
    scopes: &[String],
) -> Result<ContentLibrarySummary> {
    let values = scopes
        .iter()
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    let mut sql = String::from(
        "SELECT r.namespace,
                COUNT(*),
                COALESCE(SUM(length(COALESCE(r.payload_content, ''))), 0)
         FROM pcp_pages p
         JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
         WHERE r.namespace IN (",
    );
    push_placeholders(&mut sql, scopes.len());
    sql.push(')');
    append_current_content_page_filter(&mut sql);
    sql.push_str(" GROUP BY r.namespace ORDER BY r.namespace ASC");

    let mut statement = connection
        .prepare(&sql)
        .context("prepare PCP content library summary")?;
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .context("read PCP content library summary")?;
    let mut scopes = Vec::new();
    let mut page_count = 0_u64;
    let mut content_chars = 0_u64;
    while let Some(row) = rows.next().context("read PCP content library scope")? {
        let namespace: String = row.get(0)?;
        let scope_page_count = u64::try_from(row.get::<_, i64>(1)?)
            .context("PCP content library page count must be non-negative")?;
        let scope_content_chars = u64::try_from(row.get::<_, i64>(2)?)
            .context("PCP content library character count must be non-negative")?;
        page_count = page_count.saturating_add(scope_page_count);
        content_chars = content_chars.saturating_add(scope_content_chars);
        scopes.push(ContentLibraryScope {
            namespace,
            page_count: scope_page_count,
            content_chars: scope_content_chars,
        });
    }
    Ok(ContentLibrarySummary {
        page_count,
        content_chars,
        scopes,
    })
}

fn content_library_totals_once(
    connection: &Connection,
    scopes: &[String],
    query: Option<&str>,
) -> Result<(u64, u64)> {
    let mut values = scopes
        .iter()
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    let mut sql = String::from(
        "SELECT COUNT(*),
                COALESCE(SUM(length(COALESCE(r.payload_content, ''))), 0)
         FROM pcp_pages p
         JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
         LEFT JOIN pcp_page_summary_heads summary_head
           ON summary_head.target_page_id = p.page_id
         LEFT JOIN pcp_pages summary_page
           ON summary_page.page_id = summary_head.summary_page_id
         LEFT JOIN pcp_revisions summary_revision
           ON summary_revision.revision_id = summary_page.current_revision_id
         WHERE r.namespace IN (",
    );
    push_placeholders(&mut sql, scopes.len());
    sql.push(')');
    append_current_content_page_filter(&mut sql);
    append_content_library_query_filter(&mut sql, &mut values, query);
    let (page_count, content_chars) = connection
        .query_row(&sql, params_from_iter(values.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .context("count PCP content library")?;
    Ok((
        u64::try_from(page_count).context("PCP content library page count must be non-negative")?,
        u64::try_from(content_chars)
            .context("PCP content library character count must be non-negative")?,
    ))
}

fn append_current_content_page_filter(sql: &mut String) {
    sql.push_str(" AND r.lifecycle_status = 'active'");
    append_effective_page_filter(sql);
    sql.push_str(
        " AND NOT EXISTS (
            SELECT 1 FROM pcp_page_summary_heads summary_page_head
            WHERE summary_page_head.summary_page_id = p.page_id
        )",
    );
}

fn append_content_library_query_filter(
    sql: &mut String,
    values: &mut Vec<SqlValue>,
    query: Option<&str>,
) {
    let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    sql.push_str(
        " AND (
            instr(lower(COALESCE(r.payload_content, '')), lower(?)) > 0
            OR instr(lower(COALESCE(summary_revision.payload_content, '')), lower(?)) > 0
            OR instr(lower(COALESCE(r.facets_json, '')), lower(?)) > 0
        )",
    );
    for _ in 0..3 {
        values.push(SqlValue::Text(query.to_owned()));
    }
}

fn browse_index_order_by(order: &BrowseIndexOrder) -> &'static str {
    match order {
        BrowseIndexOrder::Recent => {
            "COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC"
        }
        BrowseIndexOrder::Oldest => "COALESCE(r.observed_at, r.created_at) ASC, r.revision_id ASC",
        BrowseIndexOrder::MostConnected => {
            "COALESCE(relation_counts.direct_relation_count, 0) DESC, \
             COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC"
        }
        BrowseIndexOrder::LeastConnected => {
            "COALESCE(relation_counts.direct_relation_count, 0) ASC, \
             COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC"
        }
        BrowseIndexOrder::Largest => {
            "length(COALESCE(r.payload_content, '')) DESC, \
             COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC"
        }
        BrowseIndexOrder::SourceOrder => {
            "CASE WHEN r.source_span_json IS NULL THEN 1 ELSE 0 END ASC, \
             json_extract(r.source_span_json, '$.streamId') ASC, \
             CAST(json_extract(r.source_span_json, '$.start') AS INTEGER) ASC, \
             CAST(json_extract(r.source_span_json, '$.end') AS INTEGER) ASC, \
             COALESCE(r.observed_at, r.created_at) ASC, r.revision_id ASC"
        }
    }
}

fn search_once(
    connection: &Connection,
    request: &SearchPagesRequest,
    mode: SearchMode,
    offset: usize,
    limit: usize,
) -> Result<SearchResult> {
    if mode == SearchMode::Graph {
        return search_graph(connection, request, offset, limit);
    }
    if mode == SearchMode::Auto {
        unreachable!();
    }
    search_surfaces(connection, request, mode, offset, limit)
}

fn search_surfaces(
    connection: &Connection,
    request: &SearchPagesRequest,
    mode: SearchMode,
    offset: usize,
    limit: usize,
) -> Result<SearchResult> {
    if mode == SearchMode::Text {
        return search_text_surfaces(connection, request, offset, limit);
    }
    let fetch_limit = offset.saturating_add(limit).saturating_add(1);
    let mut candidates = Vec::new();
    let projections = request.projections.iter().collect::<HashSet<_>>();
    if projections.contains(&Projection::Summary) {
        candidates
            .extend(search_summaries(connection, request, mode.clone(), 0, fetch_limit)?.hits);
    }
    if projections.contains(&Projection::Payload) {
        candidates.extend(
            search_revision_surface(connection, request, mode.clone(), "payload", 0, fetch_limit)?
                .hits,
        );
    }
    if projections.contains(&Projection::Facets) {
        candidates.extend(
            search_revision_surface(connection, request, mode, "facets", 0, fetch_limit)?.hits,
        );
    }
    if candidates.is_empty()
        && !request.projections.iter().any(|projection| {
            matches!(
                projection,
                Projection::Summary | Projection::Payload | Projection::Facets
            )
        })
    {
        anyhow::bail!("PCP search projections must include summary, payload, or facets");
    }

    let mut seen = HashSet::new();
    candidates.retain(|hit| seen.insert(hit.revision_id.clone()));
    let has_more = candidates.len() > offset.saturating_add(limit);
    let hits = candidates.into_iter().skip(offset).take(limit).collect();
    Ok(SearchResult {
        hits,
        next_cursor: has_more.then(|| (offset + limit).to_string()),
    })
}

fn search_text_surfaces(
    connection: &Connection,
    request: &SearchPagesRequest,
    offset: usize,
    limit: usize,
) -> Result<SearchResult> {
    // FTS scores are only meaningful within a single projection index. Recall
    // a bounded candidate pool per surface, then fuse the ranks in Rust.
    let requested = offset.saturating_add(limit).saturating_add(1);
    let fetch_limit = requested.saturating_mul(4).max(20);
    let projections = request.projections.iter().collect::<HashSet<_>>();
    let mut surfaces = Vec::<Vec<SearchHit>>::new();
    let mut source_has_more = false;

    if projections.contains(&Projection::Summary) {
        let result = search_summaries(connection, request, SearchMode::Text, 0, fetch_limit)?;
        source_has_more |= result.next_cursor.is_some();
        surfaces.push(result.hits);
    }
    if projections.contains(&Projection::Payload) {
        let result = search_revision_surface(
            connection,
            request,
            SearchMode::Text,
            "payload",
            0,
            fetch_limit,
        )?;
        source_has_more |= result.next_cursor.is_some();
        surfaces.push(result.hits);
    }
    if projections.contains(&Projection::Facets) {
        let result = search_revision_surface(
            connection,
            request,
            SearchMode::Text,
            "facets",
            0,
            fetch_limit,
        )?;
        source_has_more |= result.next_cursor.is_some();
        surfaces.push(result.hits);
    }
    if surfaces.is_empty() {
        anyhow::bail!("PCP search projections must include summary, payload, or facets");
    }

    let candidate_page_ids = surfaces
        .iter()
        .flatten()
        .map(|hit| hit.page_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let relation_pairs = active_relation_pairs(connection, &candidate_page_ids, request)?;
    let ranked = rank_text_hits(surfaces, &relation_pairs);
    let has_more = ranked.len() > offset.saturating_add(limit) || source_has_more;
    let hits = ranked.into_iter().skip(offset).take(limit).collect();
    Ok(SearchResult {
        hits,
        next_cursor: has_more.then(|| (offset + limit).to_string()),
    })
}

/// Return only direct, active Page Relations within the lexical candidate pool.
/// Provenance lives in a separate table and is deliberately excluded here.
fn active_relation_pairs(
    connection: &Connection,
    page_ids: &[String],
    request: &SearchPagesRequest,
) -> Result<Vec<RelationPair>> {
    if page_ids.len() < 2 {
        return Ok(Vec::new());
    }
    let mut sql = String::from(
        "SELECT DISTINCT relation.from_page_id, relation.to_page_id
         FROM pcp_relations relation
         WHERE relation.from_page_id IN (",
    );
    push_placeholders(&mut sql, page_ids.len());
    sql.push_str(") AND relation.to_page_id IN (");
    push_placeholders(&mut sql, page_ids.len());
    sql.push_str(
        ") AND relation.from_page_id <> relation.to_page_id
         AND NOT EXISTS (
             SELECT 1 FROM pcp_relation_retractions retraction
             WHERE retraction.relation_id = relation.relation_id
         )",
    );
    let mut values = page_ids
        .iter()
        .chain(page_ids.iter())
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    if !request.filters.relation_types.is_empty() {
        sql.push_str(" AND relation.relation_type IN (");
        push_placeholders(&mut sql, request.filters.relation_types.len());
        sql.push(')');
        values.extend(
            request
                .filters
                .relation_types
                .iter()
                .cloned()
                .map(SqlValue::Text),
        );
    }
    let mut statement = connection
        .prepare(&sql)
        .context("prepare PCP lexical relation support")?;
    statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(RelationPair {
                from_page_id: row.get(0)?,
                to_page_id: row.get(1)?,
            })
        })
        .context("query PCP lexical relation support")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect PCP lexical relation support")
}

fn search_revision_surface(
    connection: &Connection,
    request: &SearchPagesRequest,
    mode: SearchMode,
    surface: &'static str,
    offset: usize,
    limit: usize,
) -> Result<SearchResult> {
    let (content_column, matched_projection) = match surface {
        "payload" => ("r.payload_content", "payload"),
        "facets" => ("r.facets_json", "facets"),
        _ => unreachable!(),
    };
    let mut values = Vec::<SqlValue>::new();
    let mut sql = format!(
        "SELECT {REVISION_COLUMNS},
                substr(COALESCE({content_column}, ''), 1, 600),
                (
                    SELECT summary_page.current_revision_id
                    FROM pcp_page_summary_heads summary_head
                    JOIN pcp_pages summary_page
                      ON summary_page.page_id = summary_head.summary_page_id
                    WHERE summary_head.target_page_id = p.page_id
                )
         FROM pcp_pages p
         JOIN pcp_revisions r ON r.revision_id = p.current_revision_id"
    );
    if mode == SearchMode::Text {
        sql.push_str(" JOIN pcp_revision_fts ON pcp_revision_fts.revision_id = r.revision_id");
    }
    sql.push_str(" WHERE r.namespace IN (");
    push_placeholders(&mut sql, request.scopes.len());
    sql.push(')');
    values.extend(request.scopes.iter().cloned().map(SqlValue::Text));

    append_effective_page_filter(&mut sql);
    append_lifecycle_filter(&mut sql, &mut values, request);
    append_time_filters(&mut sql, &mut values, request);
    append_relation_filter(&mut sql, &mut values, request);
    match mode {
        SearchMode::Text => {
            let fts = fts_query(&request.query, &request.term_match)
                .context("text search requires at least one searchable term")?;
            sql.push_str(if surface == "payload" {
                " AND pcp_revision_fts.payload_content MATCH ?"
            } else {
                " AND pcp_revision_fts.facets_text MATCH ?"
            });
            values.push(SqlValue::Text(fts));
        }
        SearchMode::Exact => {
            if !request.query.trim().is_empty() {
                sql.push_str(&format!(
                    " AND instr(lower(COALESCE({content_column}, '')), lower(?)) > 0"
                ));
                values.push(SqlValue::Text(request.query.trim().to_owned()));
            }
        }
        SearchMode::Temporal => {
            if !request.query.trim().is_empty() {
                sql.push_str(&format!(
                    " AND instr(lower(COALESCE({content_column}, '')), lower(?)) > 0"
                ));
                values.push(SqlValue::Text(request.query.trim().to_owned()));
            }
        }
        SearchMode::Auto | SearchMode::Graph => unreachable!(),
    }
    if mode == SearchMode::Text {
        sql.push_str(
            " ORDER BY bm25(pcp_revision_fts) ASC,
                       COALESCE(r.observed_at, r.created_at) DESC,
                       r.revision_id DESC
              LIMIT ? OFFSET ?",
        );
    } else {
        sql.push_str(
            " ORDER BY COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC
              LIMIT ? OFFSET ?",
        );
    }
    values.push(SqlValue::Integer((limit + 1) as i64));
    values.push(SqlValue::Integer(offset as i64));
    collect_hits(
        connection,
        &sql,
        values,
        mode.as_str(),
        matched_projection,
        offset,
        limit,
        false,
    )
}

fn search_summaries(
    connection: &Connection,
    request: &SearchPagesRequest,
    mode: SearchMode,
    offset: usize,
    limit: usize,
) -> Result<SearchResult> {
    let fts = (mode == SearchMode::Text)
        .then(|| fts_query(&request.query, &request.term_match))
        .flatten();
    if mode == SearchMode::Text && fts.is_none() {
        anyhow::bail!("text search requires at least one searchable term");
    }
    let mut values = Vec::<SqlValue>::new();
    let mut sql = format!(
        "SELECT {REVISION_COLUMNS},
                substr(summary_revision.payload_content, 1, 600),
                summary.summary_revision_id
         FROM pcp_pages p
         JOIN pcp_revisions r ON r.revision_id = p.current_revision_id
         JOIN pcp_page_summary_heads summary_head
           ON summary_head.target_page_id = p.page_id
         JOIN pcp_pages summary_page
           ON summary_page.page_id = summary_head.summary_page_id
         JOIN pcp_summaries summary
           ON summary.summary_revision_id = summary_page.current_revision_id
         JOIN pcp_revisions summary_revision
           ON summary_revision.revision_id = summary.summary_revision_id"
    );
    if fts.is_some() {
        sql.push_str(
            "
            JOIN pcp_summary_fts
              ON pcp_summary_fts.summary_revision_id = summary.summary_revision_id",
        );
    }
    sql.push_str(" WHERE r.namespace IN (");
    push_placeholders(&mut sql, request.scopes.len());
    sql.push(')');
    values.extend(request.scopes.iter().cloned().map(SqlValue::Text));
    append_effective_page_filter(&mut sql);
    append_lifecycle_filter(&mut sql, &mut values, request);
    append_time_filters(&mut sql, &mut values, request);
    append_relation_filter(&mut sql, &mut values, request);
    if let Some(fts) = fts {
        sql.push_str(
            " AND pcp_summary_fts MATCH ?
              ORDER BY bm25(pcp_summary_fts) ASC,
                       COALESCE(r.observed_at, r.created_at) DESC,
                       r.revision_id DESC",
        );
        values.push(SqlValue::Text(fts));
    } else {
        if !request.query.trim().is_empty() {
            sql.push_str(" AND instr(lower(summary_revision.payload_content), lower(?)) > 0");
            values.push(SqlValue::Text(request.query.trim().to_owned()));
        }
        sql.push_str(" ORDER BY COALESCE(r.observed_at, r.created_at) DESC, r.revision_id DESC");
    }
    sql.push_str(" LIMIT ? OFFSET ?");
    values.push(SqlValue::Integer((limit + 1) as i64));
    values.push(SqlValue::Integer(offset as i64));
    collect_hits(
        connection,
        &sql,
        values,
        mode.as_str(),
        "summary",
        offset,
        limit,
        false,
    )
}

fn search_graph(
    connection: &Connection,
    request: &SearchPagesRequest,
    offset: usize,
    limit: usize,
) -> Result<SearchResult> {
    let query = request.query.trim();
    if query.is_empty() {
        anyhow::bail!("graph search requires a page or revision id");
    }
    let origin_revision = if query.starts_with("pg_") {
        connection
            .query_row(
                "SELECT current_revision_id FROM pcp_pages WHERE page_id = ?1",
                [query],
                |row| row.get::<_, String>(0),
            )
            .context("resolve graph search page")?
    } else {
        query.to_owned()
    };
    ensure_graph_origin_access(connection, &origin_revision, &request.scopes)?;

    let mut values = vec![SqlValue::Text(origin_revision)];
    let mut sql = String::from(
        "WITH origin AS (
            SELECT revision.page_id,
                   revision.revision_id AS requested_revision_id
            FROM pcp_revisions revision
            WHERE revision.revision_id = ?
         ),
         graph_edges (revision_id, edge_type, edge_kind, direction, basis_revision_ids_json, created_at) AS (
            SELECT
                CASE
                    WHEN relation.from_page_id = origin.page_id
                    THEN target.current_revision_id
                    ELSE source.current_revision_id
                END,
                relation.relation_type,
                'relation',
                CASE
                    WHEN relation.from_page_id = origin.page_id THEN 'outgoing'
                    ELSE 'incoming'
                END,
                COALESCE(relation.basis_revision_ids_json, '[]'),
                relation.created_at
            FROM pcp_relations relation
            JOIN origin
            JOIN pcp_pages source ON source.page_id = relation.from_page_id
            JOIN pcp_pages target ON target.page_id = relation.to_page_id
            WHERE (relation.from_page_id = origin.page_id
                   OR relation.to_page_id = origin.page_id)
              AND relation.from_page_id <> relation.to_page_id
              AND NOT EXISTS (
                  SELECT 1 FROM pcp_relation_retractions retraction
                  WHERE retraction.relation_id = relation.relation_id
              )
            UNION ALL
            SELECT
                CASE
                    WHEN provenance.derived_revision_id = origin.requested_revision_id
                    THEN provenance.input_revision_id
                    ELSE provenance.derived_revision_id
                END,
                'derived_from',
                'provenance',
                CASE
                    WHEN provenance.derived_revision_id = origin.requested_revision_id THEN 'outgoing'
                    ELSE 'incoming'
                END,
                json_array(provenance.derived_revision_id, provenance.input_revision_id),
                provenance.created_at
            FROM pcp_provenance_inputs provenance
            JOIN origin
            WHERE (provenance.derived_revision_id = origin.requested_revision_id
                   OR provenance.input_revision_id = origin.requested_revision_id)
              AND provenance.derived_revision_id <> provenance.input_revision_id
         ),
         neighbors AS (
            SELECT edge.revision_id,
                MAX(edge.created_at) AS edge_created_at,
                json_group_array(json_object(
                    'relationType', edge.edge_type,
                    'edgeKind', edge.edge_kind,
                    'direction', edge.direction,
                    'basisRevisionIds', json(edge.basis_revision_ids_json)
                )) AS graph_edges_json
            FROM graph_edges edge
            WHERE 1 = 1",
    );
    if !request.filters.relation_types.is_empty() {
        sql.push_str(" AND edge.edge_type IN (");
        push_placeholders(&mut sql, request.filters.relation_types.len());
        sql.push(')');
        values.extend(
            request
                .filters
                .relation_types
                .iter()
                .cloned()
                .map(SqlValue::Text),
        );
    }
    sql.push_str(&format!(
        "
            GROUP BY revision_id
         )
         SELECT {REVISION_COLUMNS},
                substr(COALESCE(r.payload_content, r.facets_json, ''), 1, 600),
                (
                    SELECT summary_page.current_revision_id
                    FROM pcp_page_summary_heads summary_head
                    JOIN pcp_pages summary_page
                      ON summary_page.page_id = summary_head.summary_page_id
                    WHERE summary_head.target_page_id = r.page_id
                ),
                neighbors.graph_edges_json
         FROM neighbors
         JOIN pcp_revisions r ON r.revision_id = neighbors.revision_id
         JOIN pcp_pages p ON p.page_id = r.page_id
         WHERE r.namespace IN ("
    ));
    push_placeholders(&mut sql, request.scopes.len());
    sql.push(')');
    values.extend(request.scopes.iter().cloned().map(SqlValue::Text));
    append_lifecycle_filter(&mut sql, &mut values, request);
    append_effective_page_filter(&mut sql);
    append_time_filters(&mut sql, &mut values, request);
    sql.push_str(
        " ORDER BY neighbors.edge_created_at DESC, r.revision_id DESC
          LIMIT ? OFFSET ?",
    );
    values.push(SqlValue::Integer((limit + 1) as i64));
    values.push(SqlValue::Integer(offset as i64));
    collect_hits(
        connection,
        &sql,
        values,
        "graph",
        "relations",
        offset,
        limit,
        true,
    )
}

fn ensure_graph_origin_access(
    connection: &Connection,
    revision_id: &str,
    allowed_scopes: &[String],
) -> Result<()> {
    let namespace = connection
        .query_row(
            "SELECT namespace FROM pcp_revisions WHERE revision_id = ?1",
            [revision_id],
            |row| row.get::<_, String>(0),
        )
        .with_context(|| format!("find graph origin revision {revision_id}"))?;
    if !allowed_scopes.contains(&namespace) {
        anyhow::bail!("graph origin is outside the authorized PCP scopes");
    }
    Ok(())
}

fn collect_hits(
    connection: &Connection,
    sql: &str,
    values: Vec<SqlValue>,
    matched_by: &str,
    matched_projection: &str,
    offset: usize,
    limit: usize,
    include_graph_edges: bool,
) -> Result<SearchResult> {
    let mut statement = connection.prepare(sql).context("prepare PCP search")?;
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .context("query PCP pages")?;
    let mut hits = Vec::new();
    while let Some(row) = rows.next().context("read PCP search row")? {
        let revision = revision_from_row(row, false, true, false, false)?;
        let snippet: String = row.get(17)?;
        let summary_revision_id: Option<String> = row.get(18)?;
        let graph_edges = if include_graph_edges {
            let encoded: String = row.get(19)?;
            serde_json::from_str::<Vec<GraphSearchEdge>>(&encoded)
                .context("decode PCP graph edge metadata")?
        } else {
            Vec::new()
        };
        let validity =
            current_validity(connection, &revision.revision_id)?.map(compact_validity_hint);
        let page = page_manifest(connection, &revision.page_id)?;
        hits.push(SearchHit {
            page_id: revision.page_id,
            revision_id: revision.revision_id,
            kind: page.kind,
            mutability: page.mutability,
            namespace: revision.namespace,
            lifecycle_status: revision.lifecycle_status,
            created_at: revision.created_at,
            observed_at: revision.observed_at,
            snippet,
            matched_by: matched_by.to_owned(),
            matched_projection: matched_projection.to_owned(),
            summary_revision_id,
            facets: compact_search_facets(revision.facets),
            validity,
            graph_edges,
        });
    }
    let has_more = hits.len() > limit;
    hits.truncate(limit);
    Ok(SearchResult {
        hits,
        next_cursor: has_more.then(|| (offset + limit).to_string()),
    })
}

fn compact_validity_hint(validity: PageValidity) -> PageValidityHint {
    PageValidityHint {
        assessment_page_id: validity.assessment_page_id,
        assessment_revision_id: validity.assessment_revision_id,
        standing: validity.standing,
        rationale: truncate_search_text(&validity.rationale, 360),
        scope: validity
            .scope
            .map(|scope| truncate_search_text(&scope, 240)),
        assessed_at: validity.assessed_at,
        basis_revision_count: validity.basis_revision_ids.len() as u32,
    }
}

fn truncate_search_text(content: &str, limit: usize) -> String {
    if content.chars().count() <= limit {
        return content.to_owned();
    }
    let mut compact = content
        .chars()
        .take(limit.saturating_sub(3))
        .collect::<String>();
    compact.push_str("...");
    compact
}

fn compact_search_facets(facets: Option<Value>) -> Option<Value> {
    facets.map(compact_search_value)
}

fn compact_search_value(value: Value) -> Value {
    match value {
        Value::Object(entries) => Value::Object(
            entries
                .into_iter()
                .take(24)
                .map(|(key, value)| (key, compact_search_value(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .take(12)
                .map(compact_search_value)
                .collect(),
        ),
        Value::String(content) if content.chars().count() > 240 => {
            let mut compact = content.chars().take(237).collect::<String>();
            compact.push_str("...");
            Value::String(compact)
        }
        other => other,
    }
}

fn append_lifecycle_filter(
    sql: &mut String,
    values: &mut Vec<SqlValue>,
    request: &SearchPagesRequest,
) {
    let statuses = if request.filters.lifecycle_status.is_empty() {
        vec![LifecycleStatus::Active]
    } else {
        request.filters.lifecycle_status.clone()
    };
    sql.push_str(" AND r.lifecycle_status IN (");
    push_placeholders(sql, statuses.len());
    sql.push(')');
    values.extend(
        statuses
            .into_iter()
            .map(|status| SqlValue::Text(status.as_str().to_owned())),
    );
}

fn append_effective_page_filter(sql: &mut String) {
    sql.push_str(
        " AND NOT EXISTS (
            SELECT 1 FROM pcp_relations newer
            WHERE newer.relation_type = 'supersedes'
              AND newer.to_page_id = p.page_id
              AND NOT EXISTS (
                  SELECT 1 FROM pcp_relation_retractions retraction
                  WHERE retraction.relation_id = newer.relation_id
              )
        )",
    );
}

fn append_time_filters(sql: &mut String, values: &mut Vec<SqlValue>, request: &SearchPagesRequest) {
    if let Some(created_after) = request.filters.created_after.as_ref() {
        sql.push_str(" AND COALESCE(r.observed_at, r.created_at) >= ?");
        values.push(SqlValue::Text(created_after.clone()));
    }
    if let Some(created_before) = request.filters.created_before.as_ref() {
        sql.push_str(" AND COALESCE(r.observed_at, r.created_at) <= ?");
        values.push(SqlValue::Text(created_before.clone()));
    }
}

fn append_relation_filter(
    sql: &mut String,
    values: &mut Vec<SqlValue>,
    request: &SearchPagesRequest,
) {
    if request.filters.relation_types.is_empty() {
        return;
    }
    sql.push_str(
        " AND EXISTS (
            SELECT 1 FROM pcp_relations relation_filter
            WHERE (
                relation_filter.from_page_id = p.page_id
                OR relation_filter.to_page_id = p.page_id
            ) AND NOT EXISTS (
                SELECT 1 FROM pcp_relation_retractions retraction
                WHERE retraction.relation_id = relation_filter.relation_id
            ) AND relation_filter.relation_type IN (",
    );
    push_placeholders(sql, request.filters.relation_types.len());
    sql.push_str("))");
    values.extend(
        request
            .filters
            .relation_types
            .iter()
            .cloned()
            .map(SqlValue::Text),
    );
}

fn push_placeholders(sql: &mut String, count: usize) {
    for index in 0..count {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
    }
}

fn fts_query(query: &str, term_match: &SearchTermMatch) -> Option<String> {
    let terms = query
        .split_whitespace()
        .map(|term| {
            term.chars()
                .filter(|character| character.is_alphanumeric() || *character == '_')
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    let operator = match term_match {
        SearchTermMatch::All => " AND ",
        SearchTermMatch::Any => " OR ",
    };
    (!terms.is_empty()).then(|| terms.join(operator))
}

fn parse_cursor(cursor: Option<&str>) -> Result<usize> {
    cursor
        .unwrap_or("0")
        .parse::<usize>()
        .context("invalid PCP pagination cursor")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::compact_search_facets;

    #[test]
    fn search_facets_bound_nested_routing_metadata_without_host_specific_keys() {
        let facets = compact_search_facets(Some(json!({
            "kind": "example_event",
            "parts": [{"type": "markdown", "text": "full detail"}],
            "metadata": {"snapshot": "large trace"},
            "topic": "x".repeat(300)
        })))
        .expect("compacted facets");

        assert_eq!(facets["kind"], "example_event");
        assert_eq!(facets["parts"][0]["text"], "full detail");
        assert_eq!(facets["metadata"]["snapshot"], "large trace");
        assert_eq!(
            facets["topic"]
                .as_str()
                .expect("compacted topic")
                .chars()
                .count(),
            240
        );
    }
}
