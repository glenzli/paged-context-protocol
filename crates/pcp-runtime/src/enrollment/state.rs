use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use pcp_rpc::{EnrollmentClientClaim, RequestedAccess};
use serde::{Deserialize, Serialize};

const STATE_SCHEMA: &str = "pcp.runtime.enrollment.state";
const STATE_VERSION: &str = "20260810.1";
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct EnrollmentState {
    schema: String,
    schema_version: String,
    pub requests: Vec<StoredRequest>,
    pub registrations: Vec<StoredRegistration>,
}

impl Default for EnrollmentState {
    fn default() -> Self {
        Self {
            schema: STATE_SCHEMA.to_owned(),
            schema_version: STATE_VERSION.to_owned(),
            requests: Vec::new(),
            registrations: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StoredRequest {
    pub request_id: String,
    pub client: EnrollmentClientClaim,
    pub requested_access: RequestedAccess,
    pub credential_hash: String,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub decision: StoredDecision,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum StoredDecision {
    Pending,
    Rejected,
    Approved { registration_id: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StoredRegistration {
    pub registration_id: String,
    pub client: EnrollmentClientClaim,
    pub approved_access: RequestedAccess,
    pub credential_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_opened_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub(super) struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub fn load(path: PathBuf) -> Result<(Self, EnrollmentState)> {
        let file = Self { path };
        if !file.path.exists() {
            return Ok((file, EnrollmentState::default()));
        }
        validate_state_file(&file.path)?;
        let bytes = fs::read(&file.path)
            .with_context(|| format!("read PCP enrollment state {}", file.path.display()))?;
        let state: EnrollmentState =
            serde_json::from_slice(&bytes).context("decode PCP enrollment state")?;
        anyhow::ensure!(
            state.schema == STATE_SCHEMA && state.schema_version == STATE_VERSION,
            "unsupported PCP enrollment state schema or version"
        );
        Ok((file, state))
    }

    pub fn write(&self, state: &EnrollmentState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create PCP enrollment state directory {}", parent.display())
            })?;
        }
        let temporary_path = self.path.with_file_name(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("pcp-enrollment-state"),
            std::process::id()
        ));
        let mut bytes = serde_json::to_vec_pretty(state).context("encode PCP enrollment state")?;
        bytes.push(b'\n');
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_STATE_BYTES,
            "PCP enrollment state exceeds {MAX_STATE_BYTES} bytes"
        );
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary_path)
            .with_context(|| {
                format!(
                    "open temporary PCP enrollment state {}",
                    temporary_path.display()
                )
            })?;
        output.set_permissions(fs::Permissions::from_mode(0o600))?;
        output
            .write_all(&bytes)
            .context("write PCP enrollment state")?;
        output.sync_all().context("sync PCP enrollment state")?;
        drop(output);
        fs::rename(&temporary_path, &self.path)
            .with_context(|| format!("publish PCP enrollment state {}", self.path.display()))?;
        validate_state_file(&self.path)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .context("sync PCP enrollment state directory")?;
        }
        Ok(())
    }
}

fn validate_state_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect PCP enrollment state {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "PCP enrollment state is not a regular file: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.uid() == current_uid(),
        "PCP enrollment state is not owned by the current user: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "PCP enrollment state must be mode 0600: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.len() <= MAX_STATE_BYTES,
        "PCP enrollment state exceeds {MAX_STATE_BYTES} bytes"
    );
    Ok(())
}

fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}
