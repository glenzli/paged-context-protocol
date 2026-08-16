use std::{collections::BTreeMap, path::Path, time::SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const LEDGER_RETENTION_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MaintenanceLedger {
    #[serde(default)]
    entries: BTreeMap<String, MaintenanceLedgerEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MaintenanceLedgerEntry {
    outcome: String,
    updated_at_unix_ms: u64,
    retry_after_unix_ms: u64,
}

impl MaintenanceLedger {
    pub(crate) async fn load(path: &Path) -> Result<Self> {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("decode PCP maintenance state {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => {
                Err(error).with_context(|| format!("read PCP maintenance state {}", path.display()))
            }
        }
    }

    pub(crate) async fn save(&mut self, path: &Path) -> Result<()> {
        let now = now_unix_ms();
        self.entries.retain(|_, entry| {
            entry
                .retry_after_unix_ms
                .saturating_add(LEDGER_RETENTION_MILLIS)
                >= now
        });
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!(
                    "create PCP maintenance state directory {}",
                    parent.display()
                )
            })?;
        }
        let temporary = path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(self).context("encode PCP maintenance state")?;
        tokio::fs::write(&temporary, bytes)
            .await
            .with_context(|| format!("write PCP maintenance state {}", temporary.display()))?;
        tokio::fs::rename(&temporary, path)
            .await
            .with_context(|| format!("publish PCP maintenance state {}", path.display()))
    }

    pub(crate) fn eligible(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .is_none_or(|entry| entry.retry_after_unix_ms <= now_unix_ms())
    }

    pub(crate) fn record(&mut self, key: String, outcome: &str, retry_after_seconds: u64) {
        let now = now_unix_ms();
        self.entries.insert(
            key,
            MaintenanceLedgerEntry {
                outcome: outcome.to_owned(),
                updated_at_unix_ms: now,
                retry_after_unix_ms: now.saturating_add(retry_after_seconds.saturating_mul(1_000)),
            },
        );
    }

    pub(crate) fn active_packing_sets(&self) -> Vec<Vec<String>> {
        let now = now_unix_ms();
        self.entries
            .iter()
            .filter(|(key, entry)| key.starts_with("packing:") && entry.retry_after_unix_ms > now)
            .map(|(key, _)| {
                key.trim_start_matches("packing:")
                    .split(',')
                    .map(str::to_owned)
                    .collect()
            })
            .collect()
    }

    pub(crate) fn active_relation_pairs(&self) -> Vec<[String; 2]> {
        let now = now_unix_ms();
        self.entries
            .iter()
            .filter(|(key, entry)| {
                key.starts_with("relation_pair:") && entry.retry_after_unix_ms > now
            })
            .filter_map(|(key, _)| {
                let mut page_ids = key.trim_start_matches("relation_pair:").split(',');
                let first = page_ids.next()?.to_owned();
                let second = page_ids.next()?.to_owned();
                page_ids.next().is_none().then_some([first, second])
            })
            .collect()
    }
}

pub(crate) fn summary_key(page_id: &str) -> String {
    format!("summary:{page_id}")
}

pub(crate) fn packing_key(page_ids: &[String]) -> String {
    format!("packing:{}", page_ids.join(","))
}

pub(crate) fn selection_window_key(page_ids: &[String]) -> String {
    let mut page_ids = page_ids.to_vec();
    page_ids.sort();
    page_ids.dedup();
    format!("selection_window:{}", page_ids.join(","))
}

pub(crate) fn retention_window_key(revision_ids: &[String]) -> String {
    let mut revision_ids = revision_ids.to_vec();
    revision_ids.sort();
    revision_ids.dedup();
    format!("retention_window:{}", revision_ids.join(","))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
