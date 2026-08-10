use std::{
    fs,
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use pcp_rpc::{ENROLLMENT_MAX_REQUEST_FRAME_BYTES, ENROLLMENT_MAX_RESPONSE_FRAME_BYTES};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    task::{JoinHandle, JoinSet},
    time::timeout,
};

use super::service::EnrollmentHandler;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct AdminServer {
    task: Option<JoinHandle<Result<()>>>,
    socket_path: PathBuf,
}

impl AdminServer {
    pub async fn start(socket_path: PathBuf, handler: EnrollmentHandler) -> Result<Self> {
        let listener = bind_private_socket(&socket_path, "enrollment admin").await?;
        let socket_guard = SocketFile(socket_path.clone());
        let task = tokio::spawn(async move {
            let _socket_guard = socket_guard;
            serve_admin(listener, handler).await
        });
        Ok(Self {
            task: Some(task),
            socket_path,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub async fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

impl Drop for AdminServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

async fn serve_admin(listener: UnixListener, handler: EnrollmentHandler) -> Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accept PCP enrollment admin client")?;
                connections.spawn(handle_admin_connection(stream, handler.clone()));
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("PCP enrollment admin request failed: {error:#}"),
                    Err(error) => eprintln!("PCP enrollment admin request task failed: {error}"),
                }
            }
        }
    }
}

async fn handle_admin_connection(stream: UnixStream, handler: EnrollmentHandler) -> Result<()> {
    verify_peer_user(&stream)?;
    let (read_half, mut write_half) = stream.into_split();
    let request = read_request(read_half).await?;
    let Some(request) = request else {
        return Ok(());
    };
    let response = handler.handle_admin(&request).await;
    write_response(&mut write_half, response).await
}

async fn read_request(reader: impl tokio::io::AsyncRead + Unpin) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let mut limited = BufReader::new(reader).take((ENROLLMENT_MAX_REQUEST_FRAME_BYTES + 1) as u64);
    let read = timeout(REQUEST_TIMEOUT, limited.read_until(b'\n', &mut bytes))
        .await
        .context("PCP enrollment request timed out")??;
    if read == 0 {
        return Ok(None);
    }
    anyhow::ensure!(
        bytes.len() <= ENROLLMENT_MAX_REQUEST_FRAME_BYTES && bytes.ends_with(b"\n"),
        "PCP enrollment request must be one bounded JSON line"
    );
    Ok(Some(bytes))
}

async fn write_response(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    response: Vec<u8>,
) -> Result<()> {
    anyhow::ensure!(
        response.len().saturating_add(1) <= ENROLLMENT_MAX_RESPONSE_FRAME_BYTES,
        "PCP enrollment response exceeds {ENROLLMENT_MAX_RESPONSE_FRAME_BYTES} bytes"
    );
    timeout(RESPONSE_TIMEOUT, async {
        writer.write_all(&response).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await
    })
    .await
    .context("PCP enrollment response timed out")??;
    Ok(())
}

async fn bind_private_socket(path: &Path, label: &str) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create PCP {label} socket directory {}", parent.display()))?;
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        anyhow::ensure!(
            metadata.file_type().is_socket(),
            "PCP {label} path exists and is not a socket: {}",
            path.display()
        );
        if UnixStream::connect(path).await.is_ok() {
            anyhow::bail!("another PCP {label} is listening at {}", path.display());
        }
        tokio::fs::remove_file(path)
            .await
            .with_context(|| format!("remove stale PCP {label} socket {}", path.display()))?;
    }
    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind PCP {label} socket {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("secure PCP {label} socket {}", path.display()))?;
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == current_uid()
            && metadata.permissions().mode() & 0o777 == 0o600,
        "PCP {label} socket is not owner-only: {}",
        path.display()
    );
    Ok(listener)
}

fn verify_peer_user(stream: &UnixStream) -> Result<()> {
    anyhow::ensure!(
        peer_effective_uid(stream)? == current_uid(),
        "PCP enrollment rejected a peer owned by another OS user"
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
        return Err(std::io::Error::last_os_error())
            .context("read PCP enrollment peer credentials");
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
        return Err(std::io::Error::last_os_error())
            .context("read PCP enrollment peer credentials");
    }
    anyhow::ensure!(
        length as usize == std::mem::size_of::<libc::ucred>(),
        "PCP enrollment peer credentials have an unexpected size"
    );
    // SAFETY: getsockopt succeeded and reported a complete ucred value.
    Ok(unsafe { credentials.assume_init() }.uid)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peer_effective_uid(_stream: &UnixStream) -> Result<u32> {
    anyhow::bail!("PCP enrollment peer credentials are unsupported on this platform")
}

fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

struct SocketFile(PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
