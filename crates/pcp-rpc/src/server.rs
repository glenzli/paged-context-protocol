use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::PcpApi;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinSet;

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
}

pub async fn serve_unix_endpoints(endpoints: Vec<RuntimeEndpoint>) -> Result<()> {
    anyhow::ensure!(
        !endpoints.is_empty(),
        "PCP runtime requires at least one endpoint"
    );
    let mut tasks = JoinSet::new();
    for endpoint in endpoints {
        tasks.spawn(serve_unix(endpoint.socket_path, endpoint.client));
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
    let socket_path = socket_path.as_ref().to_path_buf();
    prepare_socket_path(&socket_path).await?;
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind PCP runtime socket {}", socket_path.display()))?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("secure PCP runtime socket {}", socket_path.display()))?;
    let _guard = SocketGuard(socket_path.clone());

    loop {
        let (stream, _) = listener.accept().await.context("accept PCP RPC client")?;
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, client).await {
                eprintln!("PCP runtime connection failed: {error:#}");
            }
        });
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

async fn handle_connection(mut stream: UnixStream, client: Arc<dyn PcpApi>) -> Result<()> {
    while let Some(request) = read_frame::<RpcRequest>(&mut stream).await? {
        let id = request.id;
        let outcome = match dispatch(client.as_ref(), request.operation).await {
            Ok(value) => RpcOutcome::Ok(Box::new(value)),
            Err(error) => RpcOutcome::Error {
                message: format!("{error:#}"),
            },
        };
        write_frame(&mut stream, &RpcResponse { id, outcome }).await?;
    }
    Ok(())
}

async fn dispatch(client: &dyn PcpApi, operation: RpcOperation) -> Result<RpcValue> {
    let value = match operation {
        RpcOperation::Describe => RpcValue::Descriptor(PcpDescriptor {
            owner_id: client.owner_id().to_owned(),
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
        RpcOperation::BrowseIndex {
            scopes,
            excluded_page_kinds,
            limit,
            cursor,
            max_chars,
        } => RpcValue::SearchResult(
            client
                .browse_index(scopes, excluded_page_kinds, limit, cursor, max_chars)
                .await?,
        ),
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
        RpcOperation::WritePage(request) => {
            RpcValue::WriteResult(client.write_page(request).await?)
        }
        RpcOperation::RevisePage(request) => {
            RpcValue::WriteResult(client.revise_page(request).await?)
        }
        RpcOperation::LinkPages(request) => RpcValue::Relation(client.link_pages(request).await?),
        RpcOperation::WriteSummary(request) => {
            RpcValue::SummaryResult(client.write_summary(request).await?)
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
    };
    Ok(value)
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
