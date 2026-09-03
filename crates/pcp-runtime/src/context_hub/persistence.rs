//! Small, bounded operational state. Never stored in the Page database.
use anyhow::{Context, Result, ensure};
use pcp_client::context_hub::{CandidateInput, ClientContextPolicy};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub candidate_id: String,
    pub client_id: String,
    pub input: CandidateInput,
    pub created_at: String,
    pub expires_at: String,
    pub version: u64,
    pub status: String,
    pub snoozed_until: Option<String>,
    pub review_key: Option<String>,
    pub promotion_request: Option<pcp_client::context_hub::CandidateReview>,
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCard {
    pub card_id: String,
    pub client_id: String,
    pub scope: String,
    pub topic_key: String,
    pub summary: String,
    pub version: u64,
    pub updated_at: String,
    pub expires_at: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubState {
    pub schema_version: u32,
    pub identity_id: String,
    pub policies: Vec<ClientContextPolicy>,
    pub candidates: Vec<Candidate>,
    pub activity: Vec<ActivityCard>,
    pub sequence: u64,
}

pub struct LockedState {
    pub state: HubState,
    path: PathBuf,
    _lock: File,
}

impl LockedState {
    pub async fn open(path: &Path, identity: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path.with_extension("lock"))?;
        validate(&lock)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: valid owned descriptor; the lock is released with the file.
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            ensure!(
                error.kind() == std::io::ErrorKind::WouldBlock,
                "lock context hub: {error}"
            );
            ensure!(
                tokio::time::Instant::now() < deadline,
                "Context hub is busy; retry later with the same request"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let state = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(file) => {
                validate(&file)?;
                ensure!(
                    file.metadata()?.len() <= MAX_BYTES,
                    "context state exceeds size limit"
                );
                let mut bytes = Vec::new();
                file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= MAX_BYTES,
                    "context state exceeds size limit"
                );
                let state: HubState =
                    serde_json::from_slice(&bytes).context("decode context hub state")?;
                ensure!(
                    state.schema_version == 1 && state.identity_id == identity,
                    "context hub schema or Store identity mismatch"
                );
                state
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HubState {
                schema_version: 1,
                identity_id: identity.into(),
                ..Default::default()
            },
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            state,
            path: path.into(),
            _lock: lock,
        })
    }

    pub fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.state)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "context state exceeds size limit"
        );
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            if let Some(parent) = self.path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn prune(&mut self, now: &str) -> bool {
        let before = (self.state.activity.len(), self.state.candidates.len());
        self.state
            .activity
            .retain(|card| card.expires_at.as_str() > now);
        // In-flight promotion must retain its durable retry identity.
        self.state
            .candidates
            .retain(|item| item.expires_at.as_str() > now || item.status == "promoting");
        before != (self.state.activity.len(), self.state.candidates.len())
    }
}

fn validate(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o777 == 0o600,
        "context hub files must be current-user regular files with mode 0600"
    );
    Ok(())
}
