use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::{net::UnixListener, task::JoinHandle};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Endpoint {
    path: PathBuf,
    task: JoinHandle<()>,
    dispatched: Arc<AtomicUsize>,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn descriptor(session: &str) -> PcpDescriptor {
    PcpDescriptor {
        identity_id: "store:test".into(),
        capabilities: Capabilities {
            protocol_version: "test".into(),
            search_modes: vec![],
            projections: vec![],
            max_search_results: 1,
            max_read_pages: 1,
            max_read_chars: 100,
            features: vec![],
        },
        access: AccessSession::new(
            pcp_core::AccessPrincipal {
                principal_id: "client:test".into(),
                principal_type: pcp_core::AccessPrincipalType::ModelClient,
                display_name: None,
            },
            session,
            vec![],
        ),
        server_pid: 1,
        server_started_at_unix_ms: 1,
    }
}

fn endpoint(descriptor: PcpDescriptor, drop_response: bool, reject: bool) -> Endpoint {
    let path = PathBuf::from(format!(
        "/tmp/pcp-reconnect-{}-{}.sock",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let dispatched = Arc::new(AtomicUsize::new(0));
    let count = dispatched.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_frame::<RpcRequest>(&mut stream)
                .await
                .unwrap()
                .unwrap();
            let outcome = if matches!(request.operation, RpcOperation::Describe) {
                RpcOutcome::Ok(Box::new(RpcValue::Descriptor(descriptor.clone())))
            } else {
                count.fetch_add(1, Ordering::Relaxed);
                if drop_response {
                    continue;
                }
                if reject {
                    RpcOutcome::Error {
                        message: "permission denied".into(),
                    }
                } else if matches!(request.operation, RpcOperation::ContextHub(_)) {
                    RpcOutcome::Ok(Box::new(RpcValue::ContextHub(
                        serde_json::json!({"created":true}),
                    )))
                } else {
                    RpcOutcome::Ok(Box::new(RpcValue::Integrity("ok".into())))
                }
            };
            write_frame(
                &mut stream,
                &RpcResponse {
                    id: request.id,
                    outcome,
                },
            )
            .await
            .unwrap();
        }
    });
    Endpoint {
        path,
        task,
        dispatched,
    }
}

struct Connector {
    path: PathBuf,
    calls: AtomicUsize,
}

#[async_trait]
impl RuntimeSessionConnector for Connector {
    async fn reconnect(&self) -> Result<RemotePcpClient> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        RemotePcpClient::connect_expected(&self.path, "client:test").await
    }
}

fn connector(path: PathBuf) -> Arc<Connector> {
    Arc::new(Connector {
        path,
        calls: AtomicUsize::new(0),
    })
}

async fn submit(client: &RemotePcpClient) -> Result<serde_json::Value> {
    client
        .context_hub(pcp_client::context_hub::ContextHubRequest::SubmitCandidate(
            pcp_client::context_hub::CandidateInput {
                scope: "test".into(),
                event_id: "one".into(),
                title: "test".into(),
                content: "test".into(),
                source_refs: vec![],
                based_on_revision_ids: vec![],
            },
        ))
        .await
}

#[tokio::test]
async fn vanished_endpoint_recovers_once_across_clones_and_reports_new_session() {
    let old = endpoint(descriptor("old"), false, false);
    let new = endpoint(descriptor("new"), false, false);
    let connector = connector(new.path.clone());
    let client = RemotePcpClient::connect(&old.path)
        .await
        .unwrap()
        .with_session_connector(connector.clone());
    drop(old);
    let clone = client.clone();
    let (one, two) = tokio::join!(client.integrity_check(), clone.access_snapshot());
    assert_eq!(one.unwrap(), "ok");
    assert_eq!(two.unwrap().session_id, "new");
    assert_eq!(connector.calls.load(Ordering::Relaxed), 1);
    assert_eq!(submit(&client).await.unwrap()["created"], true);
    assert_eq!(new.dispatched.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn refused_endpoint_recovers_before_sending_a_write() {
    let old = endpoint(descriptor("old"), false, false);
    let new = endpoint(descriptor("new"), false, false);
    let connector = connector(new.path.clone());
    let client = RemotePcpClient::connect(&old.path)
        .await
        .unwrap()
        .with_session_connector(connector.clone());
    old.task.abort();
    while !old.task.is_finished() {
        tokio::task::yield_now().await;
    }
    assert!(old.path.exists());
    assert!(submit(&client).await.is_ok());
    assert_eq!(connector.calls.load(Ordering::Relaxed), 1);
    assert_eq!(new.dispatched.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn missing_write_response_and_server_rejection_are_never_replayed() {
    for (drop_response, reject) in [(true, false), (false, true)] {
        let old = endpoint(descriptor("old"), drop_response, reject);
        let new = endpoint(descriptor("new"), false, false);
        let connector = connector(new.path.clone());
        let client = RemotePcpClient::connect(&old.path)
            .await
            .unwrap()
            .with_session_connector(connector.clone());
        assert!(submit(&client).await.is_err());
        assert_eq!(old.dispatched.load(Ordering::Relaxed), 1);
        assert_eq!(new.dispatched.load(Ordering::Relaxed), 0);
        assert_eq!(connector.calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn recovery_fails_closed_on_changed_identity_permissions_or_capabilities() {
    for change in 0..4 {
        let old = endpoint(descriptor("old"), false, false);
        let mut changed = descriptor("new");
        match change {
            0 => changed.identity_id = "other-store".into(),
            1 => changed.access.principal.principal_id = "other-client".into(),
            2 => changed
                .access
                .store_permissions
                .push(pcp_core::AccessPermission::Write),
            _ => changed.capabilities.max_read_chars += 1,
        }
        let new = endpoint(changed, false, false);
        let connector = connector(new.path.clone());
        let client = RemotePcpClient::connect(&old.path)
            .await
            .unwrap()
            .with_session_connector(connector.clone());
        drop(old);
        assert!(submit(&client).await.is_err());
        assert_eq!(new.dispatched.load(Ordering::Relaxed), 0);
        assert_eq!(connector.calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn unavailable_runtime_is_bounded_and_whoami_does_not_return_cached_success() {
    let old = endpoint(descriptor("old"), false, false);
    let connector = connector(old.path.clone());
    let client = RemotePcpClient::connect(&old.path)
        .await
        .unwrap()
        .with_session_connector(connector.clone());
    drop(old);
    assert!(client.access_snapshot().await.is_err());
    assert_eq!(connector.calls.load(Ordering::Relaxed), 1);
}
