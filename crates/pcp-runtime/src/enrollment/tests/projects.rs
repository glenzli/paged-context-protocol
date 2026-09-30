//! Real enrollment transport, persisted bindings and same-session tenant writes.
use super::*;
use pcp_client::project_scope::{ProjectScopeRequest, ProjectScopeResult};
use pcp_rpc::{EnsureProjectScopeParams, ProjectRegistrationPolicyParams, RuntimeSessionConnector};

struct ProjectConnector {
    public: EnrollmentClient,
    registration_id: String,
    credential: String,
    root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl RuntimeSessionConnector for ProjectConnector {
    async fn reconnect(&self) -> anyhow::Result<RemotePcpClient> {
        let reply = self
            .public
            .open_session(OpenEnrollmentSessionParams {
                registration_id: self.registration_id.clone(),
                credential: self.credential.clone(),
            })
            .await?;
        let EnrollmentResult::Active { session } = reply.result else {
            anyhow::bail!("not active")
        };
        RemotePcpClient::connect(self.root.join(session.endpoint)).await
    }
    async fn ensure_project_scope(
        &self,
        project: ProjectScopeRequest,
    ) -> anyhow::Result<(ProjectScopeResult, RemotePcpClient)> {
        let reply = self
            .public
            .ensure_project_scope(EnsureProjectScopeParams {
                registration_id: self.registration_id.clone(),
                credential: self.credential.clone(),
                project,
            })
            .await?;
        let EnrollmentResult::ProjectReady { project, session } = reply.result else {
            anyhow::bail!("not ready")
        };
        Ok((
            project,
            RemotePcpClient::connect(self.root.join(session.endpoint)).await?,
        ))
    }
}

fn project(key: &str) -> ProjectScopeRequest {
    ProjectScopeRequest {
        project_key: key.into(),
        display_name: Some(key.into()),
        description: None,
        existing_scope: None,
    }
}

async fn enroll(
    public: &EnrollmentClient,
    admin: &EnrollmentAdminClient,
    credential: &str,
) -> pcp_rpc::EnrollmentSession {
    let EnrollmentResult::Pending { request_id, .. } =
        public.begin(begin_params(credential)).await.unwrap().result
    else {
        panic!("pending")
    };
    admin.approve(request_id.clone()).await.unwrap();
    let EnrollmentResult::Active { session } = public
        .status(EnrollmentStatusParams {
            request_id,
            credential: credential.into(),
        })
        .await
        .unwrap()
        .result
    else {
        panic!("active")
    };
    session
}

#[tokio::test]
async fn project_registration_refreshes_live_clients_and_preserves_authority() {
    let root = test_root("projects");
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .unwrap(),
    );
    let identity = store.identity_id().to_owned();
    let config = ObserverConfig::for_test(root.clone(), identity.clone());
    let enrollment = EnrollmentConfig::for_test(root.clone());
    let admin = EnrollmentAdminClient::new(&enrollment.admin_socket_path);
    let mut observer =
        ObserverService::start_with_query(config.clone(), enrollment.clone(), store.clone(), None)
            .await
            .unwrap()
            .unwrap();
    let public = EnrollmentClient::new(observer.socket_path());
    let credential = "ab".repeat(32);
    let session = enroll(&public, &admin, &credential).await;
    let registration_id = session.registration_id.clone();
    let connector = Arc::new(ProjectConnector {
        public: public.clone(),
        registration_id: registration_id.clone(),
        credential: credential.clone(),
        root: root.clone(),
    });
    let client = RemotePcpClient::connect(root.join(&session.endpoint))
        .await
        .unwrap()
        .with_session_connector(connector.clone());
    // A separate long-running process also refreshes through authenticated enrollment.
    let sibling = RemotePcpClient::connect(root.join(&session.endpoint))
        .await
        .unwrap()
        .with_session_connector(connector);
    assert!(
        client
            .ensure_project_scope(project("example.test/wiki"))
            .await
            .is_err()
    );
    assert_eq!(store.local_scope_names().await.unwrap().len(), 1);
    admin
        .project_registration_policy(ProjectRegistrationPolicyParams {
            registration_id: registration_id.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        client.ensure_project_scope(project("example.test/wiki")),
        client.ensure_project_scope(project("example.test/wiki"))
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.scope, b.scope);
    assert_ne!(a.created, b.created);
    assert_eq!(store.local_scope_names().await.unwrap().len(), 2);
    let access = client.access_snapshot().await.unwrap();
    assert!(access.allows(&a.scope, AccessPermission::Ingest));
    assert!(!access.allows(&a.scope, AccessPermission::ManageScope));
    assert!(
        sibling
            .access_snapshot()
            .await
            .unwrap()
            .allows(&a.scope, AccessPermission::Ingest)
    );
    let written = client
        .ingest_page(IngestPageRequest {
            namespace: a.scope.clone(),
            kind: "project_test".into(),
            observed_at: None,
            source_span: None,
            payload: Some(PagePayload {
                media_type: "text/plain".into(),
                content: "project evidence".into(),
            }),
            source_refs: vec![],
            based_on_revision_ids: vec![],
            facets: None,
            external_event_id: Some("project-test".into()),
        })
        .await
        .unwrap();
    let second = client
        .ensure_project_scope(project("example.test/another"))
        .await
        .unwrap();
    let access = sibling.access_snapshot().await.unwrap();
    assert!(access.allows(&a.scope, AccessPermission::Ingest));
    assert!(access.allows(&second.scope, AccessPermission::Ingest));
    let mut mismatch = project("example.test/wiki");
    mismatch.existing_scope = Some(second.scope.clone());
    assert!(client.ensure_project_scope(mismatch).await.is_err());
    let mut user = project("example.test/user");
    user.existing_scope = Some(format!("user:{identity}"));
    assert!(client.ensure_project_scope(user).await.is_err());
    assert!(
        client
            .ensure_project_scope(project("credential@example.test/wiki"))
            .await
            .is_err()
    );
    let mut missing_name = project("example.test/no-name");
    missing_name.display_name = None;
    assert!(client.ensure_project_scope(missing_name).await.is_err());
    assert!(
        public
            .ensure_project_scope(EnsureProjectScopeParams {
                registration_id: registration_id.clone(),
                credential: "ff".repeat(32),
                project: project("example.test/wiki"),
            })
            .await
            .is_err()
    );
    // A different credential cannot claim another client's project merely by knowing its key.
    let other_credential = "ef".repeat(32);
    let other = enroll(&public, &admin, &other_credential).await;
    admin
        .project_registration_policy(ProjectRegistrationPolicyParams {
            registration_id: other.registration_id.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    assert!(
        public
            .ensure_project_scope(EnsureProjectScopeParams {
                registration_id: other.registration_id,
                credential: other_credential,
                project: project("example.test/wiki"),
            })
            .await
            .is_err()
    );
    admin
        .project_registration_policy(ProjectRegistrationPolicyParams {
            registration_id: registration_id.clone(),
            enabled: false,
        })
        .await
        .unwrap();
    assert!(
        client
            .ensure_project_scope(project("example.test/wiki"))
            .await
            .is_ok()
    );
    assert!(
        client
            .ensure_project_scope(project("example.test/third"))
            .await
            .is_err()
    );
    observer.shutdown().await.unwrap();
    let mut observer = ObserverService::start_with_query(config, enrollment, store.clone(), None)
        .await
        .unwrap()
        .unwrap();
    let public = EnrollmentClient::new(observer.socket_path());
    let EnrollmentResult::Active { session } = public
        .open_session(OpenEnrollmentSessionParams {
            registration_id: registration_id.clone(),
            credential: credential.clone(),
        })
        .await
        .unwrap()
        .result
    else {
        panic!("active after restart")
    };
    assert!(session.access.allows(&a.scope, AccessPermission::Ingest));
    let rows = store
        .read_pages(
            &session.access,
            pcp_core::ReadPagesRequest {
                page_ids: vec![written.page_id],
                revision_ids: vec![],
                projections: vec![],
                max_chars: 1000,
            },
        )
        .await
        .unwrap();
    assert_eq!(rows[0].page.namespace, a.scope);
    admin.revoke(registration_id.clone()).await.unwrap();
    assert!(
        public
            .ensure_project_scope(EnsureProjectScopeParams {
                registration_id,
                credential,
                project: project("example.test/wiki"),
            })
            .await
            .is_err()
    );
    observer.shutdown().await.unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn project_registration_resumes_pending_intent_on_both_sides_of_store_creation() {
    for provisioned in [false, true] {
        let root = test_root(if provisioned {
            "after-create"
        } else {
            "before-create"
        });
        let store: Arc<dyn PcpStore> = Arc::new(
            SqlitePcpStore::open(root.join("context.sqlite3"))
                .await
                .unwrap(),
        );
        let config = ObserverConfig::for_test(root.clone(), store.identity_id());
        let enrollment = EnrollmentConfig::for_test(root.clone());
        let admin = EnrollmentAdminClient::new(&enrollment.admin_socket_path);
        let mut observer = ObserverService::start_with_query(
            config.clone(),
            enrollment.clone(),
            store.clone(),
            None,
        )
        .await
        .unwrap()
        .unwrap();
        let public = EnrollmentClient::new(observer.socket_path());
        let credential = "bc".repeat(32);
        let session = enroll(&public, &admin, &credential).await;
        admin
            .project_registration_policy(ProjectRegistrationPolicyParams {
                registration_id: session.registration_id.clone(),
                enabled: true,
            })
            .await
            .unwrap();
        observer.shutdown().await.unwrap();
        // Simulate persisted intent with no client grant, then restart the real service.
        let scope = "project:interrupted";
        let mut state: serde_json::Value =
            serde_json::from_slice(&fs::read(&enrollment.state_path).unwrap()).unwrap();
        state["projects"] = serde_json::json!([{
            "project_key": "example.test/interrupted", "scope": scope, "display_name": "Interrupted project",
            "description": null, "ready": false, "created_at": chrono::Utc::now(),
            "registration_id": session.registration_id
        }]);
        fs::write(&enrollment.state_path, serde_json::to_vec(&state).unwrap()).unwrap();
        if provisioned {
            let operator = AccessSession::full_control(
                session.access.principal.clone(),
                "test",
                vec![scope.into()],
            );
            store
                .create_scope(
                    &operator,
                    CreateScopeRequest {
                        namespace: scope.into(),
                        display_name: "Interrupted project".into(),
                        description: None,
                        parent_namespace: None,
                    },
                )
                .await
                .unwrap();
        }
        let mut observer =
            ObserverService::start_with_query(config, enrollment, store.clone(), None)
                .await
                .unwrap()
                .unwrap();
        let reply = EnrollmentClient::new(observer.socket_path())
            .ensure_project_scope(EnsureProjectScopeParams {
                registration_id: session.registration_id,
                credential,
                project: project("example.test/interrupted"),
            })
            .await
            .unwrap();
        let EnrollmentResult::ProjectReady { project, session } = reply.result else {
            panic!("ready")
        };
        assert_eq!(project.scope, scope);
        assert!(!project.created);
        assert!(session.access.allows(scope, AccessPermission::Ingest));
        assert_eq!(store.local_scope_names().await.unwrap().len(), 2);
        observer.shutdown().await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
