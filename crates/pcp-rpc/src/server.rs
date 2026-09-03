use std::{
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::PcpApi;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::{JoinHandle, JoinSet};

use crate::RuntimeQueryService;
use crate::wire::{
    PcpDescriptor, RpcOperation, RpcOutcome, RpcRequest, RpcResponse, RpcValue, read_frame,
    write_frame,
};

static SERVER_STARTED_AT_UNIX_MS: LazyLock<u64> = LazyLock::new(|| {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
});

#[derive(Clone)]
pub struct RuntimeEndpoint {
    pub socket_path: PathBuf,
    pub client: Arc<dyn PcpApi>,
    pub query_service: Option<Arc<dyn RuntimeQueryService>>,
}

pub struct RunningRuntimeEndpoint {
    socket_path: PathBuf,
    task: Option<JoinHandle<()>>,
    _guard: SocketGuard,
}

impl RunningRuntimeEndpoint {
    pub async fn start(socket_path: impl AsRef<Path>, client: Arc<dyn PcpApi>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();
        let listener = bind_unix(&socket_path).await?;
        Ok(Self::from_bound_listener(socket_path, listener, client))
    }

    pub fn from_bound_listener(
        socket_path: impl AsRef<Path>,
        listener: UnixListener,
        client: Arc<dyn PcpApi>,
    ) -> Self {
        Self::from_bound_listener_with_query(socket_path, listener, client, None)
    }

    pub fn from_bound_listener_with_query(
        socket_path: impl AsRef<Path>,
        listener: UnixListener,
        client: Arc<dyn PcpApi>,
        query_service: Option<Arc<dyn RuntimeQueryService>>,
    ) -> Self {
        let socket_path = socket_path.as_ref().to_path_buf();
        let task = tokio::spawn(async move {
            if let Err(error) = serve_listener(listener, client, query_service).await {
                eprintln!("PCP runtime endpoint failed: {error:#}");
            }
        });
        Self {
            _guard: SocketGuard(socket_path.clone()),
            socket_path,
            task: Some(task),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for RunningRuntimeEndpoint {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn serve_unix_endpoints(endpoints: Vec<RuntimeEndpoint>) -> Result<()> {
    anyhow::ensure!(
        !endpoints.is_empty(),
        "PCP runtime requires at least one endpoint"
    );
    let mut tasks = JoinSet::new();
    for endpoint in endpoints {
        tasks.spawn(serve_unix_with_query(
            endpoint.socket_path,
            endpoint.client,
            endpoint.query_service,
        ));
    }
    let outcome = tasks
        .join_next()
        .await
        .context("PCP runtime endpoint set ended unexpectedly")?;
    tasks.abort_all();
    match outcome {
        Ok(Ok(())) => anyhow::bail!("PCP runtime endpoint stopped unexpectedly"),
        Ok(Err(error)) => Err(error),
        Err(error) => Err(error).context("PCP runtime endpoint task failed"),
    }
}

pub async fn serve_unix(socket_path: impl AsRef<Path>, client: Arc<dyn PcpApi>) -> Result<()> {
    serve_unix_with_query(socket_path, client, None).await
}

pub async fn serve_unix_with_query(
    socket_path: impl AsRef<Path>,
    client: Arc<dyn PcpApi>,
    query_service: Option<Arc<dyn RuntimeQueryService>>,
) -> Result<()> {
    let socket_path = socket_path.as_ref().to_path_buf();
    let listener = bind_unix(&socket_path).await?;
    let _guard = SocketGuard(socket_path);
    serve_listener(listener, client, query_service).await
}

async fn bind_unix(socket_path: &Path) -> Result<UnixListener> {
    prepare_socket_path(socket_path).await?;
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("bind PCP runtime socket {}", socket_path.display()))?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("secure PCP runtime socket {}", socket_path.display()))?;
    Ok(listener)
}

async fn serve_listener(
    listener: UnixListener,
    client: Arc<dyn PcpApi>,
    query_service: Option<Arc<dyn RuntimeQueryService>>,
) -> Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accept PCP RPC client")?;
                let client = Arc::clone(&client);
                let query_service = query_service.clone();
                connections.spawn(handle_connection(stream, client, query_service));
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("PCP runtime connection failed: {error:#}"),
                    Err(error) => eprintln!("PCP runtime connection task failed: {error}"),
                }
            }
        }
    }
}

async fn prepare_socket_path(socket_path: &Path) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create PCP runtime directory {}", parent.display()))?;
    }
    let Ok(metadata) = std::fs::symlink_metadata(socket_path) else {
        return Ok(());
    };
    anyhow::ensure!(
        metadata.file_type().is_socket(),
        "PCP runtime path exists and is not a socket: {}",
        socket_path.display()
    );
    if UnixStream::connect(socket_path).await.is_ok() {
        anyhow::bail!(
            "another PCP runtime is already listening at {}",
            socket_path.display()
        );
    }
    tokio::fs::remove_file(socket_path)
        .await
        .with_context(|| format!("remove stale PCP runtime socket {}", socket_path.display()))?;
    Ok(())
}

async fn handle_connection(
    mut stream: UnixStream,
    client: Arc<dyn PcpApi>,
    query_service: Option<Arc<dyn RuntimeQueryService>>,
) -> Result<()> {
    verify_peer_user(&stream)?;
    while let Some(request) = read_frame::<RpcRequest>(&mut stream).await? {
        let id = request.id;
        let outcome =
            match dispatch(client.as_ref(), query_service.as_deref(), request.operation).await {
                Ok(value) => RpcOutcome::Ok(Box::new(value)),
                Err(error) => RpcOutcome::Error {
                    message: format!("{error:#}"),
                },
            };
        write_frame(&mut stream, &RpcResponse { id, outcome }).await?;
    }
    Ok(())
}

fn verify_peer_user(stream: &UnixStream) -> Result<()> {
    ensure_same_user(peer_effective_uid(stream)?, current_uid())
}

fn ensure_same_user(peer_uid: u32, expected_uid: u32) -> Result<()> {
    anyhow::ensure!(
        peer_uid == expected_uid,
        "PCP runtime rejected a peer owned by another OS user"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn peer_effective_uid(stream: &UnixStream) -> Result<u32> {
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: getpeereid writes to the two valid scalar pointers for this connected socket.
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("read PCP runtime peer credentials");
    }
    Ok(uid)
}

#[cfg(target_os = "linux")]
fn peer_effective_uid(stream: &UnixStream) -> Result<u32> {
    let mut credentials = std::mem::MaybeUninit::<libc::ucred>::zeroed();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: getsockopt writes at most length bytes into the correctly sized ucred buffer.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("read PCP runtime peer credentials");
    }
    anyhow::ensure!(
        length as usize == std::mem::size_of::<libc::ucred>(),
        "PCP runtime peer credentials have an unexpected size"
    );
    // SAFETY: getsockopt succeeded and reported a complete ucred value.
    Ok(unsafe { credentials.assume_init() }.uid)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peer_effective_uid(_stream: &UnixStream) -> Result<u32> {
    anyhow::bail!("PCP runtime peer credentials are unsupported on this platform")
}

fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

async fn dispatch(
    client: &dyn PcpApi,
    query_service: Option<&dyn RuntimeQueryService>,
    operation: RpcOperation,
) -> Result<RpcValue> {
    let value = match operation {
        RpcOperation::ContextHub(request) => {
            RpcValue::ContextHub(client.context_hub(request).await?)
        }
        RpcOperation::Describe => RpcValue::Descriptor(PcpDescriptor {
            identity_id: client.identity_id().to_owned(),
            capabilities: client.capabilities(),
            access: client.access().clone(),
            server_pid: std::process::id(),
            server_started_at_unix_ms: *SERVER_STARTED_AT_UNIX_MS,
        }),
        RpcOperation::IntegrityCheck => RpcValue::Integrity(client.integrity_check().await?),
        RpcOperation::CreateScope(request) => {
            client.create_scope(request).await?;
            RpcValue::Unit
        }
        RpcOperation::ListScopes {
            requested_scopes,
            query,
            limit,
            cursor,
        } => {
            let (scopes, next_cursor) = client
                .list_scopes(requested_scopes, query, limit, cursor)
                .await?;
            RpcValue::Scopes {
                scopes,
                next_cursor,
            }
        }
        RpcOperation::SearchPages(request) => {
            RpcValue::SearchResult(client.search_pages(request).await?)
        }
        RpcOperation::ExpandGraph(request) => {
            RpcValue::GraphSlice(pcp_client::expand_graph(client, request).await?)
        }
        RpcOperation::SemanticSearch(request) => {
            let service = query_service.context(
                "semantic_search is unavailable: this Runtime endpoint has no configured query service",
            )?;
            RpcValue::ContextQuery(service.semantic_search(client, request).await?)
        }
        RpcOperation::MatchIntent { request, effort } => {
            let service = query_service.context(
                "match_intent is unavailable: this Runtime endpoint has no configured query service",
            )?;
            RpcValue::ContextQuery(service.match_intent(client, request, effort).await?)
        }
        RpcOperation::BrowseIndex {
            scopes,
            excluded_page_kinds,
            order,
            limit,
            cursor,
            max_chars,
        } => RpcValue::SearchResult(
            client
                .browse_index(scopes, excluded_page_kinds, order, limit, cursor, max_chars)
                .await?,
        ),
        RpcOperation::BrowseContentPages {
            scopes,
            query,
            order,
            limit,
            cursor,
            max_chars,
            filter,
        } => RpcValue::ContentLibraryResult(
            client
                .browse_content_pages(scopes, query, order, limit, cursor, max_chars, filter)
                .await?,
        ),
        RpcOperation::BrowseRetrievalPages {
            scopes,
            query,
            order,
            limit,
            cursor,
            max_chars,
        } => RpcValue::ContentLibraryResult(
            client
                .browse_retrieval_pages(scopes, query, order, limit, cursor, max_chars)
                .await?,
        ),
        RpcOperation::ContentLibrarySummary { requested_scopes } => {
            RpcValue::ContentLibrarySummary(client.content_library_summary(requested_scopes).await?)
        }
        RpcOperation::ReadPages(request) => RpcValue::Pages(client.read_pages(request).await?),
        RpcOperation::CurrentRevisionId { page_id } => {
            RpcValue::RevisionId(client.current_revision_id(page_id).await?)
        }
        RpcOperation::PageCount { requested_scopes } => {
            RpcValue::PageCount(client.page_count(requested_scopes).await?)
        }
        RpcOperation::ContentCharCount { requested_scopes } => RpcValue::ContentCharCount(
            u64::try_from(client.content_char_count(requested_scopes).await?)
                .context("encode PCP content character count")?,
        ),
        RpcOperation::PlanRevisionRetention(request) => {
            RpcValue::RevisionRetentionPlan(client.plan_revision_retention(request).await?)
        }
        RpcOperation::CollectRevisionRetention(request) => {
            RpcValue::RevisionCollectionResult(client.collect_revision_retention(request).await?)
        }
        RpcOperation::PutRevisionRetentionLease(request) => {
            RpcValue::RevisionRetentionLease(client.put_revision_retention_lease(request).await?)
        }
        RpcOperation::ActiveRevisionRetentionLeases {
            requested_scopes,
            limit,
        } => RpcValue::RevisionRetentionLeases(
            client
                .active_revision_retention_leases(requested_scopes, limit)
                .await?,
        ),
        RpcOperation::IngestPage(request) => {
            RpcValue::WriteResult(client.ingest_page(request).await?)
        }
        RpcOperation::WritePage(request) => {
            RpcValue::WriteResult(client.write_page(request).await?)
        }
        RpcOperation::RevisePage(request) => {
            RpcValue::WriteResult(client.revise_page(request).await?)
        }
        RpcOperation::RepairPage(request) => {
            RpcValue::WriteResult(client.repair_page(request).await?)
        }
        RpcOperation::DeletePage(request) => {
            RpcValue::WriteResult(client.delete_page(request).await?)
        }
        RpcOperation::ArchivePage(request) => {
            RpcValue::LifecycleTransition(client.archive_page(request).await?)
        }
        RpcOperation::RestoreArchivedPage(request) => {
            RpcValue::LifecycleTransition(client.restore_archived_page(request).await?)
        }
        RpcOperation::PackPages(request) => {
            RpcValue::WriteResult(client.pack_pages(request).await?)
        }
        RpcOperation::UnpackPage(request) => {
            RpcValue::UnpackPageResult(client.unpack_page(request).await?)
        }
        RpcOperation::LinkPages(request) => RpcValue::Relation(client.link_pages(request).await?),
        RpcOperation::WriteSummary(request) => {
            RpcValue::SummaryResult(client.write_summary(request).await?)
        }
        RpcOperation::ExtractTopic(request) => {
            RpcValue::TopicExtractionResult(client.extract_topic(request).await?)
        }
        RpcOperation::NextSummaryCandidate {
            minimum_chars,
            excluded_page_kinds,
        } => RpcValue::SummaryCandidate(
            client
                .next_summary_candidate(minimum_chars, excluded_page_kinds)
                .await?,
        ),
        RpcOperation::MarkSummaryAssessed {
            target_revision_id,
            outcome,
            tool_or_model,
        } => {
            client
                .mark_summary_assessed(target_revision_id, outcome, tool_or_model)
                .await?;
            RpcValue::Unit
        }
        RpcOperation::AssessPageValidity(request) => {
            RpcValue::ValidityResult(client.assess_page_validity(request).await?)
        }
        RpcOperation::SubmitFeedback(request) => {
            RpcValue::FeedbackSubmission(client.submit_feedback(request).await?)
        }
        RpcOperation::PendingFeedback {
            requested_scopes,
            limit,
        } => RpcValue::FeedbackSignals(client.pending_feedback(requested_scopes, limit).await?),
        RpcOperation::ApplyReconciliation(request) => {
            RpcValue::ReconciliationResult(client.apply_reconciliation(request).await?)
        }
        RpcOperation::TombstoneDerivationCascade {
            root_revision_id,
            actor,
        } => RpcValue::TombstoneCascade(
            client
                .tombstone_derivation_cascade(root_revision_id, actor)
                .await?,
        ),
        RpcOperation::DurablePageInventory {
            excluded_page_kinds,
        } => RpcValue::Inventory(client.durable_page_inventory(excluded_page_kinds).await?),
        RpcOperation::AccessLog { limit, cursor } => {
            let (events, next_cursor) = client.access_log(limit, cursor).await?;
            RpcValue::AccessLog {
                events,
                next_cursor,
            }
        }
        RpcOperation::HealthSnapshot {
            requested_scopes,
            window_hours,
        } => RpcValue::HealthSnapshot(
            client
                .health_snapshot(requested_scopes, window_hours)
                .await?,
        ),
        RpcOperation::QueryAuditSummary {
            requested_scopes,
            window_hours,
        } => RpcValue::QueryAuditSummary(
            client
                .query_audit_summary(requested_scopes, window_hours)
                .await?,
        ),
    };
    Ok(value)
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_same_user;

    #[test]
    fn runtime_peer_must_have_the_provider_effective_uid() {
        assert!(ensure_same_user(501, 501).is_ok());
        assert!(ensure_same_user(502, 501).is_err());
    }
}
