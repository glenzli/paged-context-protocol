//! Approved scope policy remains live without refreshing or widening its mode.
use super::*;

#[tokio::test]
async fn approved_all_scopes_follow_new_scopes_without_operator_permissions() {
    for mode in [RequestedAccessMode::Read, RequestedAccessMode::Contribute] {
        let root = test_root("all-scopes");
        let store: Arc<dyn PcpStore> = Arc::new(
            SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .unwrap(),
        );
        let operator = AccessSession::store_wide_full_control(
            AccessPrincipal {
                principal_id: "operator:test".into(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "all-scope-fixture",
        );
        let initial = "project:existing";
        create_scope(&*store, &operator, initial).await;
        let mut config = ObserverConfig::for_test(root.clone(), store.identity_id().to_owned());
        config.enrollment_enabled = true;
        let enrollment = EnrollmentConfig::for_test(root.clone());
        let admin = EnrollmentAdminClient::new(&enrollment.admin_socket_path);
        let mut observer = ObserverService::start(config, enrollment, store.clone())
            .await
            .unwrap()
            .unwrap();
        let public = EnrollmentClient::new(observer.socket_path());
        let credential = "ab".repeat(32);
        let mut params = begin_params(&credential);
        params.client.principal.principal_type = AccessPrincipalType::ModelClient;
        params.requested_access = RequestedAccess {
            mode: mode.clone(),
            scopes: Vec::new(),
            all_scopes: true,
            read_all_scopes: false,
            allow_cross_scope_derivation: false,
        };
        let EnrollmentResult::Pending { request_id, .. } =
            public.begin(params).await.unwrap().result
        else {
            panic!("pending")
        };
        admin.approve(request_id.clone()).await.unwrap();
        let EnrollmentResult::Active { session } = public
            .status(EnrollmentStatusParams {
                request_id,
                credential,
            })
            .await
            .unwrap()
            .result
        else {
            panic!("active")
        };
        let remote = RemotePcpClient::connect(root.join(&session.endpoint))
            .await
            .unwrap();
        let expected = if mode == RequestedAccessMode::Read {
            vec![
                AccessPermission::ListScopes,
                AccessPermission::Search,
                AccessPermission::ReadSummary,
                AccessPermission::ReadDetail,
            ]
        } else {
            vec![
                AccessPermission::ListScopes,
                AccessPermission::Search,
                AccessPermission::ReadSummary,
                AccessPermission::ReadDetail,
                AccessPermission::Ingest,
            ]
        };
        assert!(remote.access().grants.is_empty());
        assert_eq!(remote.access().store_permissions.len(), expected.len());
        assert!(remote.access().has_store_permissions(&expected));
        let (scopes, _) = remote
            .list_scopes(Vec::new(), None, 20, None)
            .await
            .unwrap();
        assert!(scopes.iter().any(|scope| scope.namespace == initial));
        // This same remote endpoint was authenticated before the new Scope existed.
        let future = "project:created-after-connection";
        create_scope(&*store, &operator, future).await;
        let (scopes, _) = remote
            .list_scopes(Vec::new(), None, 20, None)
            .await
            .unwrap();
        assert!(scopes.iter().any(|scope| scope.namespace == future));
        let request = IngestPageRequest {
            namespace: future.into(),
            kind: "conversation_event".into(),
            observed_at: None,
            source_span: None,
            payload: Some(PagePayload {
                media_type: "text/plain".into(),
                content: "Scoped all-mode fixture.".into(),
            }),
            source_refs: Vec::new(),
            based_on_revision_ids: Vec::new(),
            facets: None,
            external_event_id: None,
        };
        let seeded = store.ingest_page(&operator, request.clone()).await.unwrap();
        remote
            .read_pages(pcp_core::ReadPagesRequest {
                page_ids: vec![seeded.page_id],
                revision_ids: Vec::new(),
                projections: vec![pcp_core::Projection::Payload],
                max_chars: 1000,
            })
            .await
            .unwrap();
        assert_eq!(
            remote.ingest_page(request).await.is_ok(),
            mode == RequestedAccessMode::Contribute
        );
        for permission in [
            AccessPermission::Write,
            AccessPermission::Revise,
            AccessPermission::Repair,
            AccessPermission::ManageScope,
            AccessPermission::ManageLifecycle,
            AccessPermission::Audit,
            AccessPermission::Collect,
            AccessPermission::DeriveAcrossScopes,
        ] {
            assert!(!remote.access().allows(future, permission));
        }
        assert!(
            remote
                .create_scope(CreateScopeRequest {
                    namespace: "project:forbidden".into(),
                    display_name: "forbidden".into(),
                    description: None,
                    parent_namespace: None
                })
                .await
                .is_err()
        );
        admin.revoke(session.registration_id).await.unwrap();
        assert!(
            remote
                .list_scopes(Vec::new(), None, 20, None)
                .await
                .is_err()
        );
        observer.shutdown().await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

async fn create_scope(store: &dyn PcpStore, access: &AccessSession, namespace: &str) {
    store
        .create_scope(
            access,
            CreateScopeRequest {
                namespace: namespace.into(),
                display_name: namespace.into(),
                description: None,
                parent_namespace: None,
            },
        )
        .await
        .unwrap();
}
