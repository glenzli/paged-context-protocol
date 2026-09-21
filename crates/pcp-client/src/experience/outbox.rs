//! Host-owned bounded delivery. Invoke at native checkpoints, not every model
//! turn. Exact requests survive offline/unknown outcomes; identity never drifts.
use crate::{PcpApi, context_hub::ContextHubRequest};
use anyhow::{Context, Result, ensure};
use pcp_core::AccessPermission;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PENDING: usize = 64;
const MAX_RECORD_BYTES: u64 = 32_000;
// Shorter than the Runtime's 30-day terminal candidate receipt retention.
const MAX_RETRY_AGE: u64 = 7 * 24 * 60 * 60;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    schema_version: u32,
    identity_id: String,
    principal_id: String,
    staged_at: u64,
    input: super::ExperienceCandidate,
}

pub struct ExperienceOutbox {
    root: PathBuf,
    identity: String,
    principal: String,
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn create_private(path: &Path) -> Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

impl ExperienceOutbox {
    pub fn open(root: PathBuf, identity: String, principal: String) -> Result<Self> {
        super::bounded("outbox identity", &identity, 160)?;
        super::bounded("outbox principal", &principal, 160)?;
        if !root.exists() {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&root)?;
        }
        let metadata = fs::symlink_metadata(&root)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "outbox must be a real directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "outbox must be private (0700)"
            );
        }
        Ok(Self {
            root,
            identity,
            principal,
        })
    }

    fn lock(&self) -> Result<Lock> {
        let path = self.root.join("delivery.lock");
        create_private(&path)
            .context("outbox is busy; after a crashed host, inspect the lock before removing it")?;
        Ok(Lock(path))
    }

    fn pending(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|ext| ext == "json") {
                ensure!(
                    entry.file_type()?.is_file(),
                    "outbox contains a non-regular record"
                );
                paths.push(entry.path());
                ensure!(paths.len() <= MAX_PENDING, "outbox capacity exceeded");
            }
        }
        paths.sort();
        Ok(paths)
    }

    fn read(&self, path: &Path) -> Result<Envelope> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= MAX_RECORD_BYTES,
            "invalid outbox record"
        );
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_RECORD_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES as usize,
            "outbox record exceeds budget"
        );
        let record: Envelope = serde_json::from_slice(&bytes)?;
        ensure!(
            record.schema_version == 1
                && record.identity_id == self.identity
                && record.principal_id == self.principal,
            "outbox target identity or principal changed"
        );
        Ok(record)
    }

    pub fn stage(&self, input: super::ExperienceCandidate) -> Result<String> {
        let _lock = self.lock()?;
        input.experience.validate()?;
        super::bounded("eventId", &input.candidate.event_id, 160)?;
        super::bounded("content", &input.candidate.content, 2000)?;
        let bytes = serde_json::to_vec(&(&self.identity, &self.principal, &input))?;
        let key = format!("{:x}", Sha256::digest(bytes));
        let paths = self.pending()?;
        // Matching payloads reuse the original age and request, including after restart.
        for path in &paths {
            if path
                .file_stem()
                .is_some_and(|stem| stem.to_string_lossy().ends_with(&key))
            {
                let existing = self.read(path)?;
                ensure!(
                    serde_json::to_value(&existing.input)? == serde_json::to_value(&input)?,
                    "outbox digest conflict"
                );
                return Ok(key);
            }
        }
        ensure!(
            paths.len() < MAX_PENDING,
            "outbox is full; pending evidence was retained"
        );
        let staged_at = now()?;
        let record = Envelope {
            schema_version: 1,
            identity_id: self.identity.clone(),
            principal_id: self.principal.clone(),
            staged_at,
            input,
        };
        let bytes = serde_json::to_vec(&record)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES as usize,
            "outbox record exceeds budget"
        );
        let path = self.root.join(format!("{staged_at:020}-{key}.json"));
        let temporary = self.root.join(format!("{key}.pending"));
        let mut file = create_private(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(key)
    }

    /// One bounded delivery. Errors retain the exact record and are returned to
    /// the host; no background loop, fallback principal, or content reconstruction.
    pub async fn flush_one(&self, client: &dyn PcpApi) -> Result<Option<Value>> {
        let _lock = self.lock()?;
        let paths = self.pending()?;
        let Some(path) = paths.first() else {
            return Ok(None);
        };
        let record = self.read(path)?;
        let age = now()?
            .checked_sub(record.staged_at)
            .context("outbox clock moved backwards; review pending record")?;
        ensure!(
            age <= MAX_RETRY_AGE,
            "outbox record is older than the safe retry window; compare receipts before retrying"
        );
        ensure!(
            client.identity_id() == self.identity,
            "outbox Store identity changed"
        );
        let access = client.access_snapshot().await?;
        ensure!(
            client
                .capabilities()
                .features
                .iter()
                .any(|feature| feature == crate::RUNTIME_EXPERIENCE_FEATURE),
            "Runtime does not support experience memory; exact request retained"
        );
        ensure!(
            access.principal.principal_id == self.principal
                && access.allows(&record.input.candidate.scope, AccessPermission::Ingest),
            "outbox principal or Scope permission changed"
        );
        let result = client
            .context_hub(ContextHubRequest::SubmitExperience(record.input))
            .await?;
        ensure!(
            result["candidateId"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
                && result["version"].as_u64().is_some_and(|v| v > 0)
                && result["created"].is_boolean(),
            "candidate receipt was not confirmed; exact request retained"
        );
        fs::remove_file(path)?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(Some(result))
    }
}
