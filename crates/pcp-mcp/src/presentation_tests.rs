use super::*;
use pcp_client::model_context::{ContextBudget, ContextView};
use pcp_sqlite::SqlitePcpStore;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn toolset_profiles_are_bounded_and_context_arguments_are_tenant_safe() {
    let tools = PcpMcpServer::tool_router().list_all();
    let profile = |toolset: PcpMcpToolset, context_available: bool| {
        tools
            .iter()
            .filter(|tool| toolset.exposes(tool.name.as_ref(), context_available))
            .collect::<Vec<_>>()
    };
    let core = profile(PcpMcpToolset::Core, true);
    let context = profile(PcpMcpToolset::Context, true);
    let standard = profile(PcpMcpToolset::Standard, true);
    let maintenance = profile(PcpMcpToolset::Maintenance, true);
    assert_eq!(core.len(), 5);
    assert_eq!(context.len(), 8);
    assert_eq!(standard.len(), 14);
    assert_eq!(maintenance.len(), tools.len());
    assert_eq!(profile(PcpMcpToolset::Context, false).len(), 5);
    assert_eq!(profile(PcpMcpToolset::Standard, false).len(), 11);

    let chars = serde_json::to_string(&context).unwrap().chars().count();
    let instructions = PcpMcpSurface::Codex.instructions().chars().count();
    assert!(
        chars + instructions * context.len() < 24_000,
        "catalog grew to {chars} characters plus repeated instructions"
    );
    for tool in context
        .iter()
        .filter(|tool| CONTEXT_TOOLS.contains(&tool.name.as_ref()))
    {
        assert!(
            tool.output_schema.is_some(),
            "{} has no output schema",
            tool.name
        );
        let properties = tool
            .input_schema
            .get("properties")
            .unwrap()
            .as_object()
            .unwrap();
        for forbidden in ["clientId", "principal", "actor", "observedAt"] {
            assert!(
                !properties.contains_key(forbidden),
                "{} accepts {forbidden}",
                tool.name
            );
        }
    }
    eprintln!(
        "PCP context catalog: {chars} definition characters, {instructions} instruction characters"
    );
}

#[test]
fn toolset_names_are_explicit_and_backward_compatible() {
    for (name, expected) in [
        ("core", PcpMcpToolset::Core),
        ("context", PcpMcpToolset::Context),
        ("standard", PcpMcpToolset::Standard),
        ("maintenance", PcpMcpToolset::Maintenance),
    ] {
        assert_eq!(name.parse::<PcpMcpToolset>().unwrap(), expected);
    }
    assert!("all".parse::<PcpMcpToolset>().is_err());
}

#[test]
fn literal_search_advertises_temporal_and_accepts_recent_as_a_legacy_alias() {
    let tool = PcpMcpServer::tool_router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name == "pcp_search_pages")
        .expect("search tool");
    let strategies = tool.input_schema["$defs"]["SearchStrategy"]["enum"]
        .as_array()
        .expect("strategy enum");
    assert!(strategies.contains(&json!("temporal")));
    assert!(!strategies.contains(&json!("recent")));

    let temporal: SearchPagesParams = serde_json::from_value(json!({
        "query": "test",
        "strategy": "temporal"
    }))
    .expect("canonical temporal strategy");
    assert!(matches!(temporal.strategy, SearchStrategy::Temporal));
    let recent: SearchPagesParams = serde_json::from_value(json!({
        "query": "test",
        "strategy": "recent"
    }))
    .expect("legacy recent strategy");
    assert!(matches!(recent.strategy, SearchStrategy::Temporal));
}

#[tokio::test]
async fn core_wire_is_compact_retrievable_and_permission_bound() {
    let root = std::env::temp_dir().join(format!(
        "pcp-model-wire-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(
        SqlitePcpStore::open(root.join("store.sqlite3"))
            .await
            .unwrap(),
    );
    let admin = PcpMcpServer::new(tests::full_client(
        store.clone(),
        vec!["scope:a".into(), "scope:private".into()],
    ));
    let mut revisions = vec![];
    let mut page_ids = vec![];
    for scope in ["scope:a", "scope:private"] {
        admin
            .pcp_create_scope(Parameters(CreateScopeRequest {
                namespace: scope.into(),
                display_name: scope.into(),
                description: None,
                parent_namespace: None,
            }))
            .await
            .unwrap();
        let written = admin
            .pcp_write_page(Parameters(WritePageParams {
                scope: Some(scope.into()),
                kind: "note".into(),
                mutability: PageMutability::Sealed,
                content: "# Test evidence\n\nA scoped decision, unchanged by rendering.".into(),
                based_on_revision_ids: vec![],
            }))
            .await
            .unwrap()
            .0;
        revisions.push(written.revision_id);
        page_ids.push(written.page_id);
    }
    admin
        .pcp_assess_validity(Parameters(AssessPageParams {
            target_page_id: page_ids[0].clone(),
            target_revision_id: revisions[0].clone(),
            standing: ValidityStanding::Qualified,
            rationale: "仅适用于测试版本".into(),
            evidence_revision_ids: vec![revisions[0].clone()],
        }))
        .await
        .unwrap();
    let server = PcpMcpServer::new(tests::contribute_client(store, vec!["scope:a".into()]));
    let raw = server
        .client
        .read_pages(ReadPagesRequest {
            page_ids: vec![],
            revision_ids: vec![revisions[0].clone()],
            projections: ContextView::Content.projections(),
            max_chars: 8000,
        })
        .await
        .unwrap();
    let expected =
        model_context::read_context(&raw, ContextView::Content, ContextBudget::default());
    let raw_size = serde_json::to_string(&raw).unwrap().chars().count();
    let compact_size = serde_json::to_string(&expected).unwrap().chars().count();
    assert!(compact_size < raw_size);
    let surface = server.surface_description();
    assert_eq!(surface.toolset, "core");
    assert_eq!(surface.available_tools.len(), CORE_TOOLS.len());
    assert!(
        surface
            .available_tools
            .contains(&"pcp_semantic_search".to_owned())
    );
    assert!(!surface.available_tools.contains(&"pcp_describe".to_owned()));
    let (server_io, client_io) = tokio::io::duplex(128 * 1024);
    let task = tokio::spawn(async move {
        server
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), CORE_TOOLS.len());
    assert!(tools.iter().all(|t| CORE_TOOLS.contains(&t.name.as_ref())));
    for name in ["pcp_read_pages", "pcp_search_pages", "pcp_semantic_search"] {
        assert!(
            tools
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .output_schema
                .is_none()
        );
    }
    // Cap both raw definitions and the known host's per-tool instruction expansion.
    let definition_chars = serde_json::to_string(&tools).unwrap().chars().count();
    let info = client.peer_info().unwrap();
    let instruction_chars = info.instructions.as_ref().unwrap().chars().count();
    assert!(instruction_chars <= 800);
    assert!(definition_chars + instruction_chars * tools.len() < 18_000);
    for format in ["json", "text"] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("pcp_read_pages").with_arguments(
                    json!({
                        "revisionIds":[revisions[0]], "format":format
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(result.content.len(), 1);
        assert!(
            result.structured_content.is_none(),
            "must not duplicate body"
        );
        let text = &result.content[0].as_text().unwrap().text;
        assert!(text.contains(&revisions[0]));
        assert!(text.contains("unchanged by rendering"));
        assert!(text.contains("qualified"));
        assert!(text.contains("仅适用于测试版本"));
        assert!(!text.contains("createdBy"));
        if format == "json" {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(text).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
        }
    }
    let denied = client
        .call_tool(
            CallToolRequestParams::new("pcp_read_pages").with_arguments(
                json!({
                    "revisionIds":[revisions[1]]
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    assert!(denied.is_err() || denied.unwrap().is_error == Some(true));
    let hidden = client
        .call_tool(
            CallToolRequestParams::new("pcp_create_scope").with_arguments(
                json!({
                    "namespace":"scope:unauthorized", "displayName":"unauthorized"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    assert!(hidden.is_err() || hidden.unwrap().is_error == Some(true));
    eprintln!(
        "PCP size audit: tool definitions={definition_chars}, instructions={instruction_chars}, fixture raw={raw_size}, compact={compact_size}"
    );
    drop(client);
    task.await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
