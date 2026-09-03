use super::*;
use pcp_client::model_context::{ContextBudget, ContextView};
use pcp_sqlite::SqlitePcpStore;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn optional_context_catalog_is_bounded_and_excludes_principal_arguments() {
    let mut tools = PcpMcpServer::tool_router().list_all();
    tools.retain(|tool| {
        STANDARD_TOOLS.contains(&tool.name.as_ref()) || CONTEXT_TOOLS.contains(&tool.name.as_ref())
    });
    assert_eq!(tools.len(), 14);
    let chars = serde_json::to_string(&tools).unwrap().chars().count();
    let instructions = PcpMcpSurface::Codex.instructions().chars().count();
    assert!(
        chars + instructions * tools.len() < 36_000,
        "catalog grew to {chars} characters plus repeated instructions"
    );
    for tool in tools
        .iter()
        .filter(|tool| CONTEXT_TOOLS.contains(&tool.name.as_ref()))
    {
        assert!(tool.output_schema.is_none());
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
        "PCP optional catalog: {chars} definition characters, {instructions} instruction characters"
    );
}

#[tokio::test]
async fn standard_wire_is_compact_retrievable_and_permission_bound() {
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
    assert_eq!(tools.len(), STANDARD_TOOLS.len());
    assert!(
        tools
            .iter()
            .all(|t| STANDARD_TOOLS.contains(&t.name.as_ref()))
    );
    for name in [
        "pcp_read_pages",
        "pcp_search_pages",
        "pcp_semantic_search",
        "pcp_browse_index",
    ] {
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
    assert!(definition_chars + instruction_chars * tools.len() < 30_000);
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
