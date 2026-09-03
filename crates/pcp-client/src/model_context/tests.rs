use super::*;

fn page() -> ReadPage {
    serde_json::from_value(json!({
        "page": {"pageId":"pg_a","headRevisionId":"rev_new","namespace":"scope:a",
            "kind":"note","mutability":"revisioned","lifecycleStatus":"active",
            "createdAt":"2026-09-01","updatedAt":"2026-09-03"},
        "revision": {"pageId":"pg_a","revisionId":"rev_old","namespace":"scope:a",
            "lifecycleStatus":"active","createdAt":"2026-09-01","observedAt":"2026-09-01",
            "validFrom":"2026-08-01","validTo":"2026-09-01",
            "createdBy":{"actorId":"model:internal","actorType":"model"},
            "payload":{"content":"旧观点🙂，只适用于测试。","mediaType":"text/plain"},
            "facets":{"internal":"must not reach model"},
            "sourceRefs":[{"providerId":"test","locator":"conversation:1"}],
            "provenance":[{"operation":"derive","actor":{"actorId":"internal","actorType":"model"},
                "timestamp":"2026-09-01","inputRevisionIds":["rev_basis"]}]},
        "validity":{"assessmentPageId":"pg_validity","assessmentRevisionId":"rev_validity",
            "targetPageId":"pg_a","targetRevisionId":"rev_old","standing":"qualified",
            "rationale":"仅适用于早期版本","scope":"release:old","assessedAt":"2026-09-02",
            "createdBy":{"actorId":"user","actorType":"user"}},
        "history":["rev_old","rev_new"]
    }))
    .unwrap()
}

#[test]
fn content_preserves_evidence_caveats_and_dates_without_internal_records() {
    let original = page();
    let before = serde_json::to_value(&original).unwrap();
    let result = read_context(
        &[original.clone()],
        ContextView::Content,
        ContextBudget::default(),
    );
    let value = serde_json::to_value(&result).unwrap();
    let item = &value["items"][0];
    assert_eq!(
        item["content"],
        original.revision.payload.as_ref().unwrap().content
    );
    assert_eq!(item["revisionId"], "rev_old");
    assert_eq!(item["currentRevisionId"], "rev_new");
    assert_eq!(item["observedAt"], "2026-09-01");
    assert_eq!(item["validTo"], "2026-09-01");
    assert_eq!(item["validity"]["scope"], "release:old");
    assert_eq!(item["validity"]["assessmentRevisionId"], "rev_validity");
    for field in ["facets", "createdBy", "provenance", "sourceRefs", "history"] {
        assert!(item.get(field).is_none(), "unexpected {field}");
    }
    assert_eq!(before, serde_json::to_value(&original).unwrap());
}

#[test]
fn unicode_and_upstream_truncation_are_explicit_even_with_zero_budget() {
    let result = read_context(
        &[page()],
        ContextView::Content,
        ContextBudget {
            content_chars: 4,
            preview_chars: 400,
        },
    );
    assert_eq!(result.items[0].content.as_deref(), Some("旧观点🙂"));
    assert!(result.truncated && result.items[0].truncated);
    let mut original = page();
    original.revision.payload.as_mut().unwrap().content = format!("partial\n{STORE_TRUNCATION}");
    assert!(read_context(&[original], ContextView::Content, ContextBudget::default()).truncated);
    let empty = read_context(
        &[page()],
        ContextView::Content,
        ContextBudget {
            content_chars: 0,
            preview_chars: 0,
        },
    );
    assert!(empty.truncated);
    assert_eq!(empty.items[0].revision_id, "rev_old");
}

#[test]
fn sources_and_history_are_explicit_projections() {
    let source = read_context(&[page()], ContextView::Sources, ContextBudget::default());
    assert_eq!(source.items[0].source_refs[0].locator, "conversation:1");
    assert_eq!(source.items[0].basis_revision_ids, ["rev_basis"]);
    assert!(source.items[0].content.is_none());
    assert!(source.items[0].history.is_empty());
    let history = read_context(&[page()], ContextView::History, ContextBudget::default());
    assert_eq!(history.items[0].history, ["rev_old", "rev_new"]);
    assert!(history.items[0].source_refs.is_empty());
    for view in [
        ContextView::Content,
        ContextView::Context,
        ContextView::Sources,
        ContextView::History,
        ContextView::Full,
    ] {
        assert!(view.projections().contains(&Projection::Validity));
    }
}

#[test]
fn search_preserves_pagination_summary_identity_and_graph_direction() {
    let raw: SearchResult = serde_json::from_value(json!({"hits":[{
        "pageId":"pg_a","revisionId":"rev_a","namespace":"scope:a","kind":"note",
        "mutability":"sealed","lifecycleStatus":"superseded","createdAt":"2026-09-01",
        "snippet":"一个搜索片段很长","matchedBy":"graph","matchedProjection":"summary",
        "summaryRevisionId":"rev_summary", "graphEdges":[{"relationType":"supersedes",
            "edgeKind":"relation","direction":"incoming","basisRevisionIds":["rev_basis"]}]
    }],"nextCursor":"cursor:next"}))
    .unwrap();
    let result = search_context(
        &raw,
        ContextBudget {
            content_chars: 100,
            preview_chars: 3,
        },
    );
    assert_eq!(result.next_cursor.as_deref(), Some("cursor:next"));
    assert_eq!(
        result.items[0].summary_revision_id.as_deref(),
        Some("rev_summary")
    );
    assert_eq!(result.items[0].relations[0]["direction"], "incoming");
    assert_eq!(result.items[0].page_status.as_deref(), Some("superseded"));
    assert!(result.truncated);
}

#[test]
fn text_keeps_caveats_and_ids_and_absent_validity_is_not_live() {
    let mut raw = page();
    raw.validity = None;
    let result = read_context(&[raw], ContextView::Content, ContextBudget::default());
    assert!(result.items[0].validity.is_none());
    let text = result.to_text();
    for value in ["rev_old", "rev_new", "2026-08-01", "scope:a", "旧观点🙂"] {
        assert!(text.contains(value));
    }
    assert!(!text.contains("\"live\""));
    let empty = search_context(
        &SearchResult {
            hits: vec![],
            next_cursor: None,
        },
        ContextBudget::default(),
    );
    assert!(!empty.truncated);
    assert!(empty.to_text().contains("does not establish absence"));
}

#[test]
fn query_drops_router_audit_and_marks_preview_as_excerpt() {
    let raw: QueryContextResponse = serde_json::from_value(json!({
        "scopes":["scope:a"],"visibility":"scoped","resultLimit":6,"contextBudgetChars":8000,
        "anchorCount":1,"relatedCount":0,"semanticModelCalls":12,
        "entries":[{"rank":1,"anchorRank":1,"pageId":"pg_a","revisionId":"rev_a",
            "namespace":"scope:a","kind":"note","matchedBy":"semantic","matchedProjection":"payload",
            "detail":"payload","sourceProjectionTruncated":false,"content":"这是原始内容","semanticScore":0.9}]
    })).unwrap();
    let response = query_context(
        &raw,
        ContextBudget {
            content_chars: 100,
            preview_chars: 2,
        },
    );
    assert_eq!(response.items[0].detail, "excerpt");
    assert_eq!(response.items[0].content.as_deref(), Some("这是"));
    assert!(response.truncated);
    let output = serde_json::to_string(&response).unwrap();
    assert!(!output.contains("semanticModelCalls"));
    assert!(!output.contains("semanticScore"));
}

#[test]
fn graph_keeps_bounded_edges_and_reports_incomplete_traversal() {
    let raw: GraphSliceResponse = serde_json::from_value(json!({
        "nodes":[page()], "edges":[{"fromPageId":"pg_b", "toPageId":"pg_a",
            "relationType":"supersedes", "edgeKind":"relation", "directionFromOrigin":"incoming",
            "basisRevisionIds":["rev_basis"]}], "truncated":true
    }))
    .unwrap();
    let response = graph_context(&raw, ContextView::Context, ContextBudget::default());
    assert!(response.truncated);
    assert_eq!(response.edges[0].from_page_id, "pg_b");
    assert_eq!(response.edges[0].to_page_id, "pg_a");
    assert_eq!(
        response.edges[0].direction_from_origin,
        pcp_core::GraphEdgeDirection::Incoming
    );
    assert_eq!(response.edges[0].basis_revision_ids, ["rev_basis"]);
    assert!(response.items[0].relations.is_empty());
}

#[test]
fn summary_body_is_not_mislabeled_as_original_evidence() {
    let mut read = page();
    read.revision.payload = None;
    read.summary = Some(serde_json::from_value(json!({
        "summaryPageId":"pg_summary", "summaryRevisionId":"rev_summary",
        "targetPageId":"pg_a", "targetRevisionId":"rev_old", "content":"A summary, not the body.",
        "createdAt":"2026-09-01", "createdBy":{"actorId":"test","actorType":"model"}
    })).unwrap());
    let response = read_context(&[read], ContextView::Content, ContextBudget::default());
    assert_eq!(response.items[0].detail, "summary");
    assert_eq!(
        response.items[0].summary_revision_id.as_deref(),
        Some("rev_summary")
    );
    assert_eq!(response.items[0].revision_id, "rev_old");
}

#[test]
fn related_query_entry_identifies_its_anchor() {
    let raw: QueryContextResponse = serde_json::from_value(json!({
        "scopes":["scope:a"],"visibility":"scoped","resultLimit":6,"contextBudgetChars":8000,
        "anchorCount":1,"relatedCount":1,"entries":[
            {"rank":1,"anchorRank":1,"pageId":"pg_a","revisionId":"rev_a","namespace":"scope:a",
             "kind":"note","matchedBy":"semantic","matchedProjection":"payload","detail":"payload",
             "sourceProjectionTruncated":false,"content":"Anchor"},
            {"rank":2,"anchorRank":1,"pageId":"pg_b","revisionId":"rev_b","namespace":"scope:a",
             "kind":"note","matchedBy":"graph","matchedProjection":"payload","detail":"payload",
             "sourceProjectionTruncated":false,"content":"Related","relation":{"relationType":"supersedes","direction":"incoming"}}
        ]
    })).unwrap();
    let response = query_context(&raw, ContextBudget::default());
    assert_eq!(response.items[1].anchor_page_id.as_deref(), Some("pg_a"));
    assert_eq!(response.items[1].relations[0]["direction"], "incoming");
}
