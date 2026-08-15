use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::contract::DiscoveryRegistration;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct ObserverConfig {
    pub enabled: bool,
    pub observer_enabled: bool,
    pub enrollment_enabled: bool,
    pub runtime_root: PathBuf,
    pub instance_id: String,
    pub console_url: Option<String>,
}

impl ObserverConfig {
    pub fn from_env(identity_id: &str) -> Result<Self> {
        let observer_enabled = env::var("PCP_OBSERVER_ENABLED")
            .map(|value| !matches!(value.as_str(), "0" | "false" | "no"))
            .unwrap_or(true);
        let enrollment_enabled = env::var("PCP_ENROLLMENT_ENABLED")
            .map(|value| !matches!(value.as_str(), "0" | "false" | "no"))
            .unwrap_or(true);
        let enabled = observer_enabled || enrollment_enabled;
        if !enabled {
            return Ok(Self {
                enabled,
                observer_enabled,
                enrollment_enabled,
                runtime_root: PathBuf::new(),
                instance_id: identity_id.to_owned(),
                console_url: None,
            });
        }
        let runtime_root = env::var_os("INFRA_PROTOCOL_RUNTIME_DIR")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(platform_runtime_root)?;
        anyhow::ensure!(
            runtime_root.is_absolute(),
            "INFRA_PROTOCOL_RUNTIME_DIR must be an absolute final runtime root"
        );
        validate_file_token(identity_id, "observer instance_id")?;
        Ok(Self {
            enabled,
            observer_enabled,
            enrollment_enabled,
            runtime_root,
            instance_id: identity_id.to_owned(),
            console_url: env::var("PCP_OBSERVER_CONSOLE_URL").ok(),
        })
    }

    pub fn registration_dir(&self) -> PathBuf {
        self.runtime_root.join("registrations")
    }

    pub fn socket_dir(&self) -> PathBuf {
        self.runtime_root.join("sockets")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.registration_dir()
            .join(format!("pcp--{}.json", self.instance_id))
    }

    fn authority_path(&self) -> PathBuf {
        self.registration_dir()
            .join(format!(".pcp--{}.publisher.lock", self.instance_id))
    }

    #[cfg(test)]
    pub fn for_test(runtime_root: PathBuf, instance_id: impl Into<String>) -> Self {
        Self {
            enabled: true,
            observer_enabled: true,
            enrollment_enabled: false,
            runtime_root,
            instance_id: instance_id.into(),
            console_url: Some("http://127.0.0.1:4318/".to_owned()),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn canonical_runtime_root_for_test() -> Result<PathBuf> {
    platform_runtime_root()
}

#[cfg(target_os = "macos")]
fn platform_runtime_root() -> Result<PathBuf> {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    // SAFETY: confstr writes at most the supplied buffer length and accepts a null probe buffer.
    let required =
        unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
    anyhow::ensure!(required > 1, "macOS DARWIN_USER_TEMP_DIR is unavailable");
    let mut bytes = vec![0_u8; required];
    // SAFETY: bytes is writable for required bytes, including confstr's terminating NUL.
    let written = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    anyhow::ensure!(written == required, "read macOS DARWIN_USER_TEMP_DIR");
    anyhow::ensure!(
        bytes.pop() == Some(0),
        "macOS runtime path is not NUL terminated"
    );
    let base = PathBuf::from(OsString::from_vec(bytes));
    anyhow::ensure!(
        base.is_absolute(),
        "macOS runtime directory is not absolute"
    );
    Ok(base.join("infra-protocol"))
}

#[cfg(target_os = "linux")]
fn platform_runtime_root() -> Result<PathBuf> {
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("Linux requires XDG_RUNTIME_DIR or INFRA_PROTOCOL_RUNTIME_DIR")?;
    anyhow::ensure!(base.is_absolute(), "XDG_RUNTIME_DIR must be absolute");
    validate_private_directory(&base)?;
    Ok(base.join("infra-protocol"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_runtime_root() -> Result<PathBuf> {
    anyhow::bail!("PCP Unix observer requires INFRA_PROTOCOL_RUNTIME_DIR on this platform")
}

pub fn prepare_runtime_layout(config: &ObserverConfig) -> Result<()> {
    prepare_private_directory(&config.runtime_root)?;
    prepare_private_directory(&config.registration_dir())?;
    prepare_private_directory(&config.socket_dir())?;
    Ok(())
}

fn prepare_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("create Infra Protocol directory {}", path.display()))?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect Infra Protocol directory {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Infra Protocol path is not a real directory: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.uid() == current_uid(),
        "Infra Protocol directory is not owned by the current user: {}",
        path.display()
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure Infra Protocol directory {}", path.display()))?;
    validate_private_directory(path)
}

fn validate_private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect Infra Protocol directory {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Infra Protocol path is not a real directory: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.uid() == current_uid(),
        "Infra Protocol directory is not owned by the current user: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o700,
        "Infra Protocol directory must be mode 0700: {}",
        path.display()
    );
    Ok(())
}

pub struct PublicationAuthority {
    _file: File,
}

impl PublicationAuthority {
    pub fn acquire(config: &ObserverConfig) -> Result<Self> {
        let path = config.authority_path();
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .with_context(|| format!("open PCP publication authority {}", path.display()))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file() && metadata.uid() == current_uid(),
            "PCP publication authority is not a current-user regular file: {}",
            path.display()
        );
        // SAFETY: flock operates on this owned, open file descriptor and does not dereference data.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            anyhow::bail!(
                "PCP publication authority is already held for {}: {error}",
                config.instance_id
            );
        }
        Ok(Self { _file: file })
    }
}

pub struct RegistrationFile {
    path: PathBuf,
    temporary_path: PathBuf,
}

impl RegistrationFile {
    pub fn new(path: PathBuf, generation: &str) -> Self {
        let temporary_path = path.with_file_name(format!(
            ".{}.{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("infra-discovery-registration"),
            generation
        ));
        Self {
            path,
            temporary_path,
        }
    }

    pub fn publish(&self, manifest: &DiscoveryRegistration) -> Result<()> {
        let mut bytes =
            serde_json::to_vec(manifest).context("encode Infra Discovery registration")?;
        bytes.push(b'\n');
        anyhow::ensure!(
            bytes.len() <= MAX_MANIFEST_BYTES,
            "Infra Discovery registration exceeds {MAX_MANIFEST_BYTES} bytes"
        );
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.temporary_path)
            .with_context(|| {
                format!(
                    "open temporary Infra Discovery registration {}",
                    self.temporary_path.display()
                )
            })?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)
            .context("write Infra Discovery registration")?;
        file.sync_all()
            .context("sync Infra Discovery registration")?;
        drop(file);
        fs::rename(&self.temporary_path, &self.path).with_context(|| {
            format!(
                "publish Infra Discovery registration {}",
                self.path.display()
            )
        })?;
        validate_private_manifest(&self.path)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .context("sync Infra Discovery registration directory")?;
        }
        Ok(())
    }
}

impl Drop for RegistrationFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary_path);
    }
}

fn validate_private_manifest(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect Infra Discovery registration {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Infra Discovery registration is not a regular file: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.uid() == current_uid(),
        "Infra Discovery registration is not owned by the current user: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "Infra Discovery registration must be mode 0600: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.len() <= MAX_MANIFEST_BYTES as u64,
        "Infra Discovery registration exceeds {MAX_MANIFEST_BYTES} bytes"
    );
    Ok(())
}

fn validate_file_token(value: &str, field: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 96,
        "{field} has invalid length"
    );
    let mut bytes = value.bytes();
    anyhow::ensure!(
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "{field} contains unsupported filename characters"
    );
    Ok(())
}

pub(crate) fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::validate_file_token;

    #[test]
    fn discovery_file_tokens_match_the_schema_pattern() {
        for valid in ["owner_123", "proc.123", "A-b"] {
            assert!(validate_file_token(valid, "test token").is_ok());
        }
        for invalid in ["", "_owner", ".owner", "-owner", "owner/path", "owner name"] {
            assert!(validate_file_token(invalid, "test token").is_err());
        }
        assert!(validate_file_token(&"a".repeat(96), "test token").is_ok());
        assert!(validate_file_token(&"a".repeat(97), "test token").is_err());
    }
}
