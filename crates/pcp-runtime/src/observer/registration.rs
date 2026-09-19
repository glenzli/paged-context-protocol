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
    path: PathBuf,
    identity: (u64, u64),
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
            metadata.is_file()
                && metadata.uid() == current_uid()
                && metadata.permissions().mode() & 0o777 == 0o600,
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
        Ok(Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
            _file: file,
        })
    }

    fn ensure_current(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.path).with_context(|| {
            format!("inspect PCP publication authority {}", self.path.display())
        })?;
        anyhow::ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == current_uid()
                && metadata.permissions().mode() & 0o777 == 0o600
                && (metadata.dev(), metadata.ino()) == self.identity,
            "PCP publication authority changed at {}; restart the owning Runtime",
            self.path.display()
        );
        Ok(())
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
        let bytes = encoded_manifest(manifest)?;
        self.write_temporary(&bytes)?;
        fs::rename(&self.temporary_path, &self.path).with_context(|| {
            format!(
                "publish Infra Discovery registration {}",
                self.path.display()
            )
        })?;
        self.validate_and_sync(&bytes)
    }

    pub fn ensure_published(
        &self,
        authority: &PublicationAuthority,
        manifest: &DiscoveryRegistration,
    ) -> Result<bool> {
        let registration_dir = self
            .path
            .parent()
            .context("Infra Discovery registration has no parent directory")?;
        let runtime_root = registration_dir
            .parent()
            .context("Infra Discovery registration directory has no runtime root")?;
        validate_private_directory(runtime_root)?;
        validate_private_directory(registration_dir)?;
        authority.ensure_current()?;
        let expected = encoded_manifest(manifest)?;
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {
                self.validate_exact(&expected)?;
                Ok(false)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.write_temporary(&expected)?;
                if let Err(error) = authority.ensure_current() {
                    let _ = fs::remove_file(&self.temporary_path);
                    return Err(error);
                }
                let linked = match fs::hard_link(&self.temporary_path, &self.path) {
                    Ok(()) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                    Err(error) => {
                        let _ = fs::remove_file(&self.temporary_path);
                        return Err(error).with_context(|| {
                            format!(
                                "restore Infra Discovery registration {}",
                                self.path.display()
                            )
                        });
                    }
                };
                let _ = fs::remove_file(&self.temporary_path);
                self.validate_and_sync(&expected)?;
                Ok(linked)
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "inspect Infra Discovery registration {}",
                    self.path.display()
                )
            }),
        }
    }

    fn write_temporary(&self, bytes: &[u8]) -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
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
        Ok(())
    }

    fn validate_exact(&self, expected: &[u8]) -> Result<()> {
        validate_private_manifest(&self.path)?;
        let current = fs::read(&self.path).with_context(|| {
            format!("read Infra Discovery registration {}", self.path.display())
        })?;
        anyhow::ensure!(
            current == expected,
            "Infra Discovery registration changed at {}; restart the owning Runtime",
            self.path.display()
        );
        Ok(())
    }

    fn validate_and_sync(&self, expected: &[u8]) -> Result<()> {
        self.validate_exact(expected)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .context("sync Infra Discovery registration directory")?;
        }
        Ok(())
    }
}

fn encoded_manifest(manifest: &DiscoveryRegistration) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(manifest).context("encode Infra Discovery registration")?;
    bytes.push(b'\n');
    anyhow::ensure!(
        bytes.len() <= MAX_MANIFEST_BYTES,
        "Infra Discovery registration exceeds {MAX_MANIFEST_BYTES} bytes"
    );
    Ok(bytes)
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
    use std::{fs, os::unix::fs::MetadataExt};

    use uuid::Uuid;

    use super::{
        ObserverConfig, PublicationAuthority, RegistrationFile, prepare_runtime_layout,
        validate_file_token,
    };
    use crate::observer::contract::{
        DISCOVERY_REGISTRATION_SCHEMA, DISCOVERY_SCHEMA_VERSION, DiscoveryOffer,
        DiscoveryRegistration, DiscoveryService, LOCAL_UNIX_SOCKET_BINDING,
        PCP_OBSERVER_PROTOCOL_ID, PCP_OBSERVER_PROTOCOL_VERSION,
    };

    fn manifest(generation: &str) -> DiscoveryRegistration {
        DiscoveryRegistration {
            schema: DISCOVERY_REGISTRATION_SCHEMA.to_owned(),
            schema_version: DISCOVERY_SCHEMA_VERSION.to_owned(),
            service: DiscoveryService {
                kind: "pcp".to_owned(),
                instance_id: "pcp-test".to_owned(),
                generation: generation.to_owned(),
            },
            offers: vec![DiscoveryOffer {
                protocol: PCP_OBSERVER_PROTOCOL_ID.to_owned(),
                protocol_versions: vec![PCP_OBSERVER_PROTOCOL_VERSION.to_owned()],
                binding: LOCAL_UNIX_SOCKET_BINDING.to_owned(),
                endpoint: "sockets/PCPTEST000000001.sock".to_owned(),
            }],
        }
    }

    fn test_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pcp-registration-{name}-{}",
            Uuid::new_v4().simple()
        ))
    }

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

    #[test]
    fn missing_registration_is_restored_without_rewriting_healthy_content() {
        let root = test_root("restore");
        let config = ObserverConfig::for_test(root.join("infra-protocol"), "pcp-test");
        prepare_runtime_layout(&config).unwrap();
        let authority = PublicationAuthority::acquire(&config).unwrap();
        let registration = RegistrationFile::new(config.manifest_path(), "proc_test");
        let expected = manifest("proc_test");
        registration.publish(&expected).unwrap();
        let original = fs::metadata(&registration.path).unwrap();

        assert!(
            !registration
                .ensure_published(&authority, &expected)
                .unwrap()
        );
        let unchanged = fs::metadata(&registration.path).unwrap();
        assert_eq!(original.ino(), unchanged.ino());
        assert_eq!(original.modified().unwrap(), unchanged.modified().unwrap());

        fs::remove_file(&registration.path).unwrap();
        assert!(
            registration
                .ensure_published(&authority, &expected)
                .unwrap()
        );
        assert!(
            !registration
                .ensure_published(&authority, &expected)
                .unwrap()
        );
        drop(registration);
        drop(authority);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repair_preserves_conflicts_and_rejects_replaced_authority() {
        let root = test_root("conflict");
        let config = ObserverConfig::for_test(root.join("infra-protocol"), "pcp-test");
        prepare_runtime_layout(&config).unwrap();
        let authority = PublicationAuthority::acquire(&config).unwrap();
        let registration = RegistrationFile::new(config.manifest_path(), "proc_test");
        let expected = manifest("proc_test");
        registration.publish(&expected).unwrap();

        fs::write(&registration.path, b"{}\n").unwrap();
        assert!(
            registration
                .ensure_published(&authority, &expected)
                .is_err()
        );
        assert_eq!(fs::read(&registration.path).unwrap(), b"{}\n");

        fs::remove_file(config.authority_path()).unwrap();
        let successor = PublicationAuthority::acquire(&config).unwrap();
        fs::remove_file(&registration.path).unwrap();
        assert!(
            registration
                .ensure_published(&authority, &expected)
                .is_err()
        );
        assert!(!registration.path.exists());
        assert!(
            registration
                .ensure_published(&successor, &expected)
                .unwrap()
        );
        drop(registration);
        drop(successor);
        drop(authority);
        fs::remove_dir_all(root).unwrap();
    }
}
