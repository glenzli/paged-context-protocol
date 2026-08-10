use std::{
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use tokio::net::UnixListener;
use uuid::Uuid;

const SOCKET_ID_BYTES: usize = 16;
const BIND_ATTEMPTS: usize = 16;

pub(crate) struct BoundInfraSocket {
    endpoint: String,
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    cleanup_on_drop: bool,
}

impl BoundInfraSocket {
    pub(crate) fn bind(runtime_root: &Path) -> Result<Self> {
        bind_with(runtime_root, || compact_socket_id(&Uuid::new_v4()))
    }

    pub(crate) fn into_parts(mut self) -> (String, PathBuf, UnixListener) {
        self.cleanup_on_drop = false;
        let listener = self
            .listener
            .take()
            .expect("bound Infra socket listener must be present");
        (self.endpoint.clone(), self.socket_path.clone(), listener)
    }
}

impl Drop for BoundInfraSocket {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

fn bind_with(
    runtime_root: &Path,
    mut next_socket_id: impl FnMut() -> String,
) -> Result<BoundInfraSocket> {
    for _ in 0..BIND_ATTEMPTS {
        let endpoint = format!("sockets/{}.sock", next_socket_id());
        let socket_path = validate_endpoint_path(runtime_root, &endpoint)?;
        let listener = match UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            Err(error) if is_address_collision(&error) => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("bind PCP Infra socket {}", socket_path.display()));
            }
        };

        if let Err(error) = secure_bound_socket(&socket_path) {
            drop(listener);
            let _ = fs::remove_file(&socket_path);
            return Err(error);
        }
        return Ok(BoundInfraSocket {
            endpoint,
            socket_path,
            listener: Some(listener),
            cleanup_on_drop: true,
        });
    }
    anyhow::bail!("could not allocate a unique PCP Infra socket after {BIND_ATTEMPTS} attempts")
}

fn validate_endpoint_path(runtime_root: &Path, endpoint: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        runtime_root.is_absolute(),
        "PCP Infra runtime root must be absolute"
    );
    let opaque = endpoint
        .strip_prefix("sockets/")
        .and_then(|value| value.strip_suffix(".sock"))
        .context("PCP Infra socket endpoint is not canonical")?;
    anyhow::ensure!(
        !opaque.is_empty()
            && opaque.len() <= SOCKET_ID_BYTES
            && opaque
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && opaque
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') }),
        "PCP Infra socket endpoint has an invalid opaque ID"
    );

    let socket_dir = runtime_root.join("sockets");
    let socket_path = socket_dir.join(format!("{opaque}.sock"));
    anyhow::ensure!(
        socket_path.parent() == Some(socket_dir.as_path()),
        "PCP Infra socket endpoint escapes the sockets directory"
    );
    let encoded_path = socket_path.as_os_str().as_bytes();
    anyhow::ensure!(
        !encoded_path.contains(&0),
        "PCP Infra socket path contains an embedded NUL"
    );
    let capacity = unix_socket_path_capacity();
    let required = encoded_path.len().saturating_add(1);
    anyhow::ensure!(
        required <= capacity,
        "PCP Infra socket path requires {required} bytes including NUL, but this platform allows {capacity}: {}",
        socket_path.display()
    );
    Ok(socket_path)
}

fn secure_bound_socket(socket_path: &Path) -> Result<()> {
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("secure PCP Infra socket {}", socket_path.display()))?;
    let metadata = fs::symlink_metadata(socket_path)
        .with_context(|| format!("inspect PCP Infra socket {}", socket_path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_socket() && metadata.uid() == current_uid(),
        "PCP Infra endpoint is not a current-user Unix socket: {}",
        socket_path.display()
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "PCP Infra socket must be mode 0600: {}",
        socket_path.display()
    );
    Ok(())
}

fn unix_socket_path_capacity() -> usize {
    // SAFETY: sockaddr_un is a plain C struct and all-zero is a valid initialization.
    let address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    address.sun_path.len()
}

fn is_address_collision(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::AlreadyExists
    )
}

fn compact_socket_id(id: &Uuid) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    const ENTROPY_INDICES: [usize; 10] = [0, 1, 2, 3, 4, 5, 9, 10, 11, 12];

    let bytes = id.as_bytes();
    let entropy = ENTROPY_INDICES.map(|index| bytes[index]);
    let mut encoded = String::with_capacity(SOCKET_ID_BYTES);
    for bit_offset in (0..80).step_by(5) {
        let byte_index = bit_offset / 8;
        let bit_in_byte = bit_offset % 8;
        let pair = u16::from(entropy[byte_index]) << 8
            | u16::from(entropy.get(byte_index + 1).copied().unwrap_or(0));
        let value = (pair >> (11 - bit_in_byte)) & 0x1f;
        encoded.push(ALPHABET[value as usize] as char);
    }
    encoded
}

fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::{ffi::OsStrExt, fs::PermissionsExt},
        path::Path,
    };

    use tokio::net::UnixListener;
    use uuid::Uuid;

    use super::{bind_with, compact_socket_id, unix_socket_path_capacity, validate_endpoint_path};

    #[test]
    fn socket_id_is_canonical_and_uses_the_full_budget() {
        let socket_id = compact_socket_id(&Uuid::nil());
        assert_eq!(socket_id.len(), 16);
        assert!(socket_id.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }

    #[test]
    fn endpoint_path_accounts_for_the_terminating_nul() {
        let capacity = unix_socket_path_capacity();
        let endpoint = "sockets/0123456789abcdef.sock";
        let fitting_root = format!("/{}", "r".repeat(capacity - endpoint.len() - 3));
        let path = validate_endpoint_path(Path::new(&fitting_root), endpoint)
            .expect("boundary path should fit");
        assert_eq!(path.as_os_str().as_bytes().len() + 1, capacity);

        let oversized_root = format!("/{}", "r".repeat(capacity - endpoint.len() - 2));
        assert!(validate_endpoint_path(Path::new(&oversized_root), endpoint).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn generated_endpoint_fits_the_canonical_macos_runtime_root() {
        let root = crate::observer::canonical_runtime_root_for_test()
            .expect("resolve canonical macOS Infra runtime root");
        let endpoint = format!("sockets/{}.sock", compact_socket_id(&Uuid::nil()));
        validate_endpoint_path(&root, &endpoint).expect("canonical macOS endpoint should fit");
    }

    #[test]
    fn endpoint_rejects_noncanonical_opaque_ids() {
        let root = Path::new("/tmp/infra-protocol");
        for endpoint in [
            "sockets/.sock",
            "sockets/_opaque.sock",
            "sockets/0123456789abcdefg.sock",
            "sockets/nested/name.sock",
        ] {
            assert!(
                validate_endpoint_path(root, endpoint).is_err(),
                "{endpoint}"
            );
        }
    }

    #[tokio::test]
    async fn binding_retries_after_an_address_collision() {
        let root = Path::new("/tmp").join(format!(
            "pcp-infra-socket-collision-{}",
            Uuid::new_v4().simple()
        ));
        let socket_dir = root.join("sockets");
        fs::create_dir_all(&socket_dir).expect("create socket test directory");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("secure test root");
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700))
            .expect("secure test socket directory");

        let occupied_id = "0000000000000000";
        let replacement_id = "1111111111111111";
        let occupied_path = socket_dir.join(format!("{occupied_id}.sock"));
        let occupied = UnixListener::bind(&occupied_path).expect("bind occupied socket");
        let mut candidates = [occupied_id, replacement_id].into_iter();
        let bound = bind_with(&root, || {
            candidates.next().expect("socket candidate").to_owned()
        })
        .expect("retry socket binding");
        assert_eq!(bound.endpoint, format!("sockets/{replacement_id}.sock"));

        drop(bound);
        drop(occupied);
        fs::remove_file(occupied_path).expect("remove occupied socket");
        fs::remove_dir_all(root).expect("remove socket test root");
    }
}
