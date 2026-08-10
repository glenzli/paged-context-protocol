use std::{
    fs,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use pcp_store::PcpStore;
use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::{MissedTickBehavior, timeout},
};
use uuid::Uuid;

use super::{
    contract::{
        DISCOVERY_REGISTRATION_SCHEMA, DISCOVERY_SCHEMA_VERSION, DiscoveryLease, DiscoveryOffer,
        DiscoveryRegistration, DiscoveryService, ERROR_SCHEMA, LOCAL_UNIX_SOCKET_BINDING,
        ObserverError, PCP_OBSERVER_PROTOCOL_ID, PCP_OBSERVER_PROTOCOL_VERSION, REQUEST_SCHEMA,
        SnapshotRequest,
    },
    registration::{
        ObserverConfig, PublicationAuthority, RegistrationFile, current_uid, prepare_runtime_layout,
    },
    snapshot::SnapshotSource,
};

const SERVICE_KIND: &str = "pcp";
const INTEGRITY_INTERVAL: Duration = Duration::from_secs(10 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST_FRAME_BYTES: usize = 4 * 1024;
const MAX_RESPONSE_FRAME_BYTES: usize = 1024 * 1024;

pub struct ObserverService {
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
    socket_path: PathBuf,
    generation: String,
}

impl ObserverService {
    pub async fn start(config: ObserverConfig, store: Arc<dyn PcpStore>) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        prepare_runtime_layout(&config)?;
        let authority = PublicationAuthority::acquire(&config)?;
        let generation_id = Uuid::new_v4();
        let generation = format!("proc_{}", generation_id.simple());
        let endpoint = config.socket_endpoint(&compact_socket_id(&generation_id))?;
        let socket_path = config.socket_path(&endpoint);
        validate_socket_path_length(&socket_path)?;
        prepare_socket_path(&socket_path).await?;
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("bind PCP observer socket {}", socket_path.display()))?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("secure PCP observer socket {}", socket_path.display()))?;
        validate_private_socket(&socket_path)?;
        let socket_file = SocketFile(socket_path.clone());

        let source = Arc::new(SnapshotSource::new(
            Arc::clone(&store),
            config.instance_id.clone(),
            generation.clone(),
            config.console_url.clone(),
        ));
        let registration = RegistrationFile::new(config.manifest_path(), &generation);
        registration.write(&discovery_registration(&config, &generation, &endpoint)?)?;

        let task_config = config.clone();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_observer(
            listener,
            socket_file,
            authority,
            registration,
            task_config,
            endpoint,
            source,
            shutdown_rx,
        ));
        Ok(Some(Self {
            shutdown,
            task: Some(task),
            socket_path,
            generation,
        }))
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn wait(&mut self) -> Result<()> {
        let result = self
            .task
            .as_mut()
            .context("PCP observer task is not running")?
            .await;
        self.task = None;
        result.context("join PCP observer task")?
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.take() {
            task.await.context("join PCP observer shutdown")??;
        }
        self.cleanup_socket();
        Ok(())
    }

    fn cleanup_socket(&self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

impl Drop for ObserverService {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.cleanup_socket();
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_observer(
    listener: UnixListener,
    _socket: SocketFile,
    _authority: PublicationAuthority,
    registration: RegistrationFile,
    config: ObserverConfig,
    endpoint: String,
    source: Arc<SnapshotSource>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut connections = JoinSet::new();
    let mut integrity_task = tokio::spawn(refresh_integrity(Arc::clone(&source), shutdown.clone()));
    let mut integrity_finished = false;
    let mut renewal = tokio::time::interval(config.renew_interval);
    renewal.set_missed_tick_behavior(MissedTickBehavior::Delay);
    renewal.tick().await;

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accept PCP observer client")?;
                let source = Arc::clone(&source);
                connections.spawn(async move { handle_connection(stream, source).await });
            }
            _ = renewal.tick() => {
                registration.write(&discovery_registration(
                    &config,
                    source.generation(),
                    &endpoint,
                )?)?;
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("PCP observer request failed: {error:#}"),
                    Err(error) => eprintln!("PCP observer request task failed: {error}"),
                }
            }
            result = &mut integrity_task => {
                integrity_finished = true;
                match result {
                    Ok(Ok(())) if *shutdown.borrow() => break,
                    Ok(Ok(())) => anyhow::bail!("PCP observer integrity task stopped unexpectedly"),
                    Ok(Err(error)) => return Err(error).context("PCP observer integrity task failed"),
                    Err(error) => return Err(error).context("join PCP observer integrity task"),
                }
            }
        }
    }

    connections.abort_all();
    if !integrity_finished {
        integrity_task.abort();
        let _ = integrity_task.await;
    }
    drop(registration);
    Ok(())
}

async fn refresh_integrity(
    source: Arc<SnapshotSource>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    loop {
        source.refresh_integrity().await;
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = tokio::time::sleep(INTEGRITY_INTERVAL) => {}
        }
    }
}

async fn handle_connection(stream: UnixStream, source: Arc<SnapshotSource>) -> Result<()> {
    verify_peer_user(&stream)?;
    let (read_half, mut write_half) = stream.into_split();
    let mut request_bytes = Vec::new();
    let mut limited = BufReader::new(read_half).take((MAX_REQUEST_FRAME_BYTES + 1) as u64);
    let read = timeout(
        REQUEST_TIMEOUT,
        limited.read_until(b'\n', &mut request_bytes),
    )
    .await
    .context("PCP observer request timed out")??;
    if read == 0 {
        return Ok(());
    }
    let response =
        if request_bytes.len() > MAX_REQUEST_FRAME_BYTES || !request_bytes.ends_with(b"\n") {
            encode_error("invalid_request", "request must be one bounded JSON line")?
        } else {
            match validate_request(&request_bytes) {
                Ok(()) => match source.capture().await {
                    Ok(snapshot) => encode_response(&snapshot)?,
                    Err(_) => encode_error(
                        "snapshot_unavailable",
                        "snapshot is temporarily unavailable",
                    )?,
                },
                Err(error) => encode_error("invalid_request", &error)?,
            }
        };
    let response = if response.len().saturating_add(1) <= MAX_RESPONSE_FRAME_BYTES {
        response
    } else {
        encode_error(
            "response_too_large",
            "snapshot exceeds the response frame limit",
        )?
    };
    anyhow::ensure!(
        response.len().saturating_add(1) <= MAX_RESPONSE_FRAME_BYTES,
        "PCP observer error exceeds {MAX_RESPONSE_FRAME_BYTES} bytes"
    );
    timeout(RESPONSE_TIMEOUT, async {
        write_half.write_all(&response).await?;
        write_half.write_all(b"\n").await?;
        Ok::<(), std::io::Error>(())
    })
    .await
    .context("PCP observer response timed out")??;
    Ok(())
}

fn validate_request(bytes: &[u8]) -> std::result::Result<(), String> {
    let request = serde_json::from_slice::<SnapshotRequest>(bytes)
        .map_err(|_| "request is not valid PCP observer JSON".to_owned())?;
    if request.schema != REQUEST_SCHEMA
        || request.schema_version != PCP_OBSERVER_PROTOCOL_VERSION
        || request.operation != "snapshot"
    {
        return Err("unsupported PCP observer request schema, version, or operation".to_owned());
    }
    Ok(())
}

fn encode_response(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).context("encode PCP observer response")
}

fn encode_error(code: &str, message: &str) -> Result<Vec<u8>> {
    encode_response(&ObserverError {
        schema: ERROR_SCHEMA.to_owned(),
        schema_version: PCP_OBSERVER_PROTOCOL_VERSION.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

fn discovery_registration(
    config: &ObserverConfig,
    generation: &str,
    endpoint: &str,
) -> Result<DiscoveryRegistration> {
    let renewed = Utc::now();
    let expires = renewed
        + chrono::Duration::from_std(config.lease_ttl)
            .context("convert PCP discovery lease TTL")?;
    Ok(DiscoveryRegistration {
        schema: DISCOVERY_REGISTRATION_SCHEMA.to_owned(),
        schema_version: DISCOVERY_SCHEMA_VERSION.to_owned(),
        service: DiscoveryService {
            kind: SERVICE_KIND.to_owned(),
            instance_id: config.instance_id.clone(),
            generation: generation.to_owned(),
        },
        lease: DiscoveryLease {
            renewed_at: renewed.to_rfc3339_opts(SecondsFormat::Millis, true),
            expires_at: expires.to_rfc3339_opts(SecondsFormat::Millis, true),
        },
        offers: vec![DiscoveryOffer {
            protocol: PCP_OBSERVER_PROTOCOL_ID.to_owned(),
            protocol_versions: vec![PCP_OBSERVER_PROTOCOL_VERSION.to_owned()],
            binding: LOCAL_UNIX_SOCKET_BINDING.to_owned(),
            endpoint: endpoint.to_owned(),
        }],
    })
}

fn verify_peer_user(stream: &UnixStream) -> Result<()> {
    let peer_uid = peer_effective_uid(stream)?;
    ensure_same_user(peer_uid, current_uid())
}

pub(super) fn ensure_same_user(peer_uid: u32, expected_uid: u32) -> Result<()> {
    anyhow::ensure!(
        peer_uid == expected_uid,
        "PCP observer rejected a peer owned by another OS user"
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
        return Err(std::io::Error::last_os_error()).context("read PCP observer peer credentials");
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
        return Err(std::io::Error::last_os_error()).context("read PCP observer peer credentials");
    }
    anyhow::ensure!(
        length as usize == std::mem::size_of::<libc::ucred>(),
        "PCP observer peer credentials have an unexpected size"
    );
    // SAFETY: getsockopt succeeded and reported a complete ucred value.
    Ok(unsafe { credentials.assume_init() }.uid)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peer_effective_uid(_stream: &UnixStream) -> Result<u32> {
    anyhow::bail!("PCP observer peer credentials are unsupported on this platform")
}

async fn prepare_socket_path(path: &Path) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    anyhow::ensure!(
        metadata.file_type().is_socket(),
        "PCP observer path exists and is not a socket: {}",
        path.display()
    );
    if UnixStream::connect(path).await.is_ok() {
        anyhow::bail!("a PCP observer is already listening at {}", path.display());
    }
    tokio::fs::remove_file(path)
        .await
        .with_context(|| format!("remove stale PCP observer socket {}", path.display()))?;
    Ok(())
}

fn validate_private_socket(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect PCP observer socket {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_socket() && metadata.uid() == current_uid(),
        "PCP observer endpoint is not a current-user Unix socket: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "PCP observer socket must be mode 0600: {}",
        path.display()
    );
    Ok(())
}

fn validate_socket_path_length(path: &Path) -> Result<()> {
    // SAFETY: sockaddr_un is a plain C struct and all-zero is a valid initialization.
    let address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    let capacity = address.sun_path.len();
    anyhow::ensure!(
        path.as_os_str().as_bytes().len() < capacity,
        "PCP observer socket path is too long for this platform (must be under {capacity} bytes): {}",
        path.display()
    );
    Ok(())
}

fn compact_socket_id(id: &Uuid) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = id.as_bytes();
    let mut encoded = String::with_capacity(23);
    encoded.push('p');
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0b11) << 4) | second.unwrap_or(0) >> 4) as usize] as char);
        if let Some(second) = second {
            encoded.push(
                ALPHABET[(((second & 0b1111) << 2) | third.unwrap_or(0) >> 6) as usize] as char,
            );
        }
        if let Some(third) = third {
            encoded.push(ALPHABET[(third & 0b111111) as usize] as char);
        }
    }
    encoded
}

struct SocketFile(PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
