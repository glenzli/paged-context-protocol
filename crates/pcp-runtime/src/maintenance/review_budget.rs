//! Cross-process admission and exactly-once settlement for upgraded reviews.
//! Kept separate from maintenance snapshots: every mutation reloads under lock.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

const WINDOW_MS: u64 = 86_400_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewBudgetConfig {
    pub enabled: bool,
    pub state_path: PathBuf,
    pub sol_deployment_id: String,
    pub astra_deployment_id: String,
    pub sol_max_calls: u32,
    pub sol_max_tokens: u64,
    pub astra_enabled: bool,
    pub astra_max_calls: u32,
    pub astra_max_tokens: Option<u64>,
    pub max_in_flight: u32,
    pub max_input_bytes: usize,
    /// Output allowance for admission reservations only, never a provider cap.
    pub max_output_tokens: u32,
    /// Budgets gate new calls; admitted calls finish without a budget output cap.
    /// Actual usage can exceed the final reservation.
    pub token_limit_mode: TokenLimitMode,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenLimitMode {
    #[default]
    #[serde(alias = "hard")]
    Admission,
}
impl Default for ReviewBudgetConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            state_path: PathBuf::new(),
            sol_deployment_id: "codex_gpt_5_6_sol".into(),
            astra_deployment_id: "codex_gpt_6_astra".into(),
            sol_max_calls: 300,
            sol_max_tokens: 6_000_000,
            astra_enabled: true,
            astra_max_calls: 10,
            astra_max_tokens: None,
            max_in_flight: 1,
            max_input_bytes: 512_000,
            max_output_tokens: 8192,
            token_limit_mode: TokenLimitMode::Admission,
        }
    }
}
impl ReviewBudgetConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.sol_deployment_id.trim().is_empty()
                && !self.astra_deployment_id.trim().is_empty(),
            "review deployments must not be empty"
        );
        ensure!(
            self.sol_max_calls > 0 && self.sol_max_tokens > 0 && self.astra_max_calls > 0,
            "review budgets must be positive"
        );
        ensure!(
            self.astra_max_tokens.is_none_or(|n| n > 0),
            "Astra token budget must be positive"
        );
        ensure!(
            (1..=4).contains(&self.max_in_flight)
                && (1024..=2_000_000).contains(&self.max_input_bytes)
                && (256..=32768).contains(&self.max_output_tokens),
            "invalid review request limits"
        );
        Ok(())
    }
    pub fn snapshot(&self) -> Result<BudgetSnapshot> {
        BudgetStore::new(self.clone()).snapshot()
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTier {
    Sol,
    Astra,
}
impl ReviewTier {
    pub fn effort(self) -> &'static str {
        match self {
            Self::Sol => "high",
            Self::Astra => "low",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAttempt {
    pub key: String,
    pub evidence_key: String,
    pub request_hash: String,
    pub tier: ReviewTier,
    pub stage: String,
    pub deployment: String,
    pub effort: String,
    pub submitted_at_ms: u64,
    pub reserved_tokens: u64,
    pub actual_tokens: Option<u64>,
    pub response_id: Option<String>,
    pub state: String,
    pub reason: String,
    pub result: Option<Value>,
}
#[derive(Default, Deserialize, Serialize)]
struct BudgetLedger {
    #[serde(default)]
    clock_ms: u64,
    #[serde(default)]
    attempts: BTreeMap<String, ReviewAttempt>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierBudgetStatus {
    pub tier: ReviewTier,
    pub max_calls: u32,
    pub max_tokens: Option<u64>,
    pub used_calls: u32,
    pub actual_tokens: u64,
    pub reserved_tokens: u64,
    pub remaining_calls: u32,
    pub remaining_tokens: Option<u64>,
    pub next_release_at_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSnapshot {
    pub enabled: bool,
    pub token_limit_mode: TokenLimitMode,
    pub window_seconds: u64,
    pub in_flight: u32,
    pub sol: TierBudgetStatus,
    pub astra: TierBudgetStatus,
    pub attempts: Vec<ReviewAttempt>,
}
pub(crate) enum Admission {
    Reserved(ReviewAttempt),
    Existing(ReviewAttempt),
    Waiting(String),
}
#[derive(Clone)]
pub(crate) struct BudgetStore {
    config: ReviewBudgetConfig,
}
impl BudgetStore {
    pub fn new(config: ReviewBudgetConfig) -> Self {
        Self { config }
    }
    fn transaction<T>(&self, f: impl FnOnce(&mut BudgetLedger, u64) -> Result<T>) -> Result<T> {
        self.access(true, f)
    }
    fn inspect<T>(&self, f: impl FnOnce(&mut BudgetLedger, u64) -> Result<T>) -> Result<T> {
        self.access(false, f)
    }
    fn access<T>(
        &self,
        write: bool,
        f: impl FnOnce(&mut BudgetLedger, u64) -> Result<T>,
    ) -> Result<T> {
        ensure!(
            !self.config.state_path.as_os_str().is_empty(),
            "review budget state path unavailable"
        );
        let path = &self.config.state_path;
        let parent = path.parent().context("review budget parent missing")?;
        std::fs::create_dir_all(parent)?;
        let _lock = FileLock::acquire(&path.with_extension("lock"))?;
        let mut ledger = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<BudgetLedger>(&bytes)
                .context("review budget is corrupt; refusing to reset consumption")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BudgetLedger::default(),
            Err(e) => return Err(e.into()),
        };
        let now = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .min(u64::MAX as u128) as u64)
            .max(ledger.clock_ms);
        ledger.clock_ms = now;
        let result = f(&mut ledger, now)?;
        if write {
            atomic_write(path, &serde_json::to_vec(&ledger)?)?;
        }
        Ok(result)
    }
    pub fn reserve(
        &self,
        evidence: &str,
        request_hash: &str,
        tier: ReviewTier,
        stage: &str,
        tokens: u64,
        reason: &str,
    ) -> Result<Admission> {
        let key = hash(&format!("{evidence}:{tier:?}:{stage}"));
        self.transaction(|ledger, now| {
            if let Some(old) = ledger.attempts.get(&key) {
                return Ok(Admission::Existing(old.clone()));
            }
            let active = ledger.attempts.values().filter(|a| unresolved(a)).count();
            if active >= self.config.max_in_flight as usize {
                return Ok(Admission::Waiting(
                    "Another upgraded review has an unsettled reservation".into(),
                ));
            }
            let status = self.tier_status(ledger, now, tier);
            if status.remaining_calls == 0 || status.remaining_tokens.is_some_and(|n| n < tokens) {
                return Ok(Admission::Waiting(
                    "Rolling review budget exhausted; waiting for reservations to settle or expire"
                        .into(),
                ));
            }
            let attempt = ReviewAttempt {
                key: key.clone(),
                evidence_key: evidence.into(),
                request_hash: request_hash.into(),
                tier,
                stage: stage.into(),
                deployment: match tier {
                    ReviewTier::Sol => self.config.sol_deployment_id.clone(),
                    ReviewTier::Astra => self.config.astra_deployment_id.clone(),
                },
                effort: tier.effort().into(),
                submitted_at_ms: now,
                reserved_tokens: tokens,
                actual_tokens: None,
                response_id: None,
                state: "reserved".into(),
                reason: reason.into(),
                result: None,
            };
            ledger.attempts.insert(key, attempt.clone());
            Ok(Admission::Reserved(attempt))
        })
    }
    pub fn submitted(&self, key: &str, id: &str) -> Result<()> {
        self.transaction(|l, _| {
            let a = l.attempts.get_mut(key).context("missing reservation")?;
            ensure!(
                a.response_id.as_deref().is_none_or(|old| old == id),
                "response identity changed"
            );
            a.response_id = Some(id.into());
            a.state = "in_progress".into();
            Ok(())
        })
    }
    pub fn settle(
        &self,
        key: &str,
        tokens: Option<u64>,
        result: Option<Value>,
        state: &str,
    ) -> Result<()> {
        self.transaction(|l, _| {
            let a = l.attempts.get_mut(key).context("missing reservation")?;
            if a.actual_tokens.is_some() {
                return Ok(());
            }
            a.actual_tokens = tokens;
            a.result = result;
            a.state = if tokens.is_some() {
                state.into()
            } else {
                "usage_unknown".into()
            };
            Ok(())
        })
    }
    pub fn evidence_seen(&self, key: &str) -> Result<bool> {
        self.inspect(|l, _| Ok(l.attempts.values().any(|a| a.evidence_key == key)))
    }
    pub fn unsettled(&self) -> Result<Vec<ReviewAttempt>> {
        self.inspect(|l, _| {
            Ok(l.attempts
                .values()
                .filter(|a| unresolved(a))
                .cloned()
                .collect())
        })
    }
    pub fn attempt(&self, key: &str) -> Result<Option<ReviewAttempt>> {
        self.inspect(|l, _| Ok(l.attempts.get(key).cloned()))
    }
    pub fn snapshot(&self) -> Result<BudgetSnapshot> {
        self.inspect(|l, now| {
            let mut attempts = l.attempts.values().cloned().collect::<Vec<_>>();
            attempts.sort_by_key(|a| std::cmp::Reverse(a.submitted_at_ms));
            attempts.truncate(100);
            for a in &mut attempts {
                a.result = None;
            }
            Ok(BudgetSnapshot {
                enabled: self.config.enabled,
                token_limit_mode: self.config.token_limit_mode,
                window_seconds: WINDOW_MS / 1000,
                in_flight: l.attempts.values().filter(|a| unresolved(a)).count() as u32,
                sol: self.tier_status(l, now, ReviewTier::Sol),
                astra: self.tier_status(l, now, ReviewTier::Astra),
                attempts,
            })
        })
    }
    fn tier_status(&self, l: &BudgetLedger, now: u64, tier: ReviewTier) -> TierBudgetStatus {
        let (max_calls, max_tokens) = match tier {
            ReviewTier::Sol => (self.config.sol_max_calls, Some(self.config.sol_max_tokens)),
            ReviewTier::Astra => (self.config.astra_max_calls, self.config.astra_max_tokens),
        };
        let mut used_calls = 0u32;
        let mut actual = 0u64;
        let mut reserved = 0u64;
        let mut next = None;
        for a in l.attempts.values().filter(|a| {
            a.tier == tier
                && a.state != "not_submitted"
                && (unresolved(a) || now.saturating_sub(a.submitted_at_ms) < WINDOW_MS)
        }) {
            used_calls = used_calls.saturating_add(1);
            if let Some(n) = a.actual_tokens {
                actual = actual.saturating_add(n);
                let at = a.submitted_at_ms.saturating_add(WINDOW_MS);
                next = Some(next.map_or(at, |old: u64| old.min(at)));
            } else {
                reserved = reserved.saturating_add(a.reserved_tokens);
            }
        }
        TierBudgetStatus {
            tier,
            max_calls,
            max_tokens,
            used_calls,
            actual_tokens: actual,
            reserved_tokens: reserved,
            remaining_calls: max_calls.saturating_sub(used_calls),
            remaining_tokens: max_tokens.map(|n| n.saturating_sub(actual.saturating_add(reserved))),
            next_release_at_ms: next,
        }
    }
}
fn unresolved(a: &ReviewAttempt) -> bool {
    a.actual_tokens.is_none()
}
pub(crate) fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
struct FileLock(File);
impl FileLock {
    fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
            "lock review budget: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(file))
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDir(PathBuf);
    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn store() -> (TestDir, BudgetStore) {
        let dir = TestDir(
            std::env::temp_dir().join(format!("pcp-review-budget-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir_all(dir.path()).unwrap();
        let store = BudgetStore::new(ReviewBudgetConfig {
            enabled: true,
            state_path: dir.path().join("budget.json"),
            sol_max_calls: 1,
            sol_max_tokens: 100,
            ..Default::default()
        });
        (dir, store)
    }
    fn reserve(s: &BudgetStore, id: &str) -> Admission {
        s.reserve(id, "proposal", ReviewTier::Sol, "review", 80, "test")
            .unwrap()
    }
    #[test]
    fn defaults_and_legacy_hard_config_use_admission() {
        assert_eq!(
            ReviewBudgetConfig::default().token_limit_mode,
            TokenLimitMode::Admission
        );
        let config: ReviewBudgetConfig =
            serde_json::from_value(serde_json::json!({"token_limit_mode": "hard"})).unwrap();
        assert_eq!(config.token_limit_mode, TokenLimitMode::Admission);
        assert_eq!(
            serde_json::to_value(config).unwrap()["token_limit_mode"],
            "admission"
        );
    }

    #[test]
    fn completed_call_may_exceed_threshold_and_blocks_next_admission() {
        let (_dir, mut s) = store();
        s.config.sol_max_calls = 10;
        let Admission::Reserved(a) = reserve(&s, "first") else {
            panic!()
        };
        s.settle(
            &a.key,
            Some(150),
            Some(serde_json::json!({"output": "complete"})),
            "completed",
        )
        .unwrap();
        let reopened = BudgetStore::new(s.config.clone());
        let status = reopened.snapshot().unwrap();
        assert_eq!(status.sol.actual_tokens, 150);
        assert_eq!(status.sol.remaining_tokens, Some(0));
        assert_eq!(status.sol.remaining_calls, 9);
        assert_eq!(status.in_flight, 0);
        assert!(matches!(reserve(&reopened, "next"), Admission::Waiting(_)));
        let Admission::Existing(completed) = reserve(&reopened, "first") else {
            panic!()
        };
        assert_eq!(completed.state, "completed");
        assert_eq!(completed.result.unwrap()["output"], "complete");
    }

    #[test]
    fn concurrent_last_slot_and_duplicate_are_not_double_charged() {
        let (_dir, s) = store();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles = (0..2)
            .map(|i| {
                let s = s.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    matches!(reserve(&s, &i.to_string()), Admission::Reserved(_))
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        assert_eq!(
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .filter(|v| *v)
                .count(),
            1
        );
        let snapshot = s.snapshot().unwrap();
        assert_eq!(snapshot.sol.used_calls, 1);
        assert_eq!(snapshot.sol.reserved_tokens, 80);
        let a = &snapshot.attempts[0];
        assert!(matches!(
            reserve(&s, &a.evidence_key),
            Admission::Existing(_)
        ));
        s.settle(&a.key, Some(30), None, "completed").unwrap();
        s.settle(&a.key, Some(70), None, "completed").unwrap();
        assert_eq!(s.snapshot().unwrap().sol.actual_tokens, 30);
    }
    #[test]
    fn unknown_usage_survives_restart_and_window_until_reconciled() {
        let (_dir, s) = store();
        let Admission::Reserved(a) = reserve(&s, "evidence") else {
            panic!()
        };
        s.submitted(&a.key, "response-1").unwrap();
        s.settle(&a.key, None, None, "cancelled").unwrap();
        s.transaction(|l, now| {
            l.attempts.get_mut(&a.key).unwrap().submitted_at_ms = now - WINDOW_MS - 1;
            Ok(())
        })
        .unwrap();
        let reopened = BudgetStore::new(s.config.clone());
        assert_eq!(reopened.snapshot().unwrap().in_flight, 1);
        assert!(matches!(reserve(&reopened, "new"), Admission::Waiting(_)));
        reopened
            .settle(&a.key, Some(99), None, "cancelled")
            .unwrap();
        assert_eq!(reopened.snapshot().unwrap().sol.used_calls, 0);
        assert!(matches!(reserve(&reopened, "new"), Admission::Reserved(_)));
    }
    #[test]
    fn corrupt_state_fails_closed_and_clock_rollback_does_not_release() {
        let (dir, s) = store();
        reserve(&s, "old");
        s.transaction(|l, now| {
            l.clock_ms = now + WINDOW_MS * 2;
            for a in l.attempts.values_mut() {
                a.submitted_at_ms = l.clock_ms;
                a.actual_tokens = Some(80);
            }
            Ok(())
        })
        .unwrap();
        assert!(matches!(reserve(&s, "new"), Admission::Waiting(_)));
        std::fs::write(dir.path().join("budget.json"), b"broken").unwrap();
        assert!(s.snapshot().is_err());
        assert!(
            s.reserve("new", "p", ReviewTier::Sol, "review", 1, "test")
                .is_err()
        );
    }
    #[test]
    fn token_ceiling_blocks_even_with_calls_remaining_and_snapshots_hide_results() {
        let (_dir, mut s) = store();
        s.config.sol_max_calls = 10;
        let Admission::Reserved(a) = reserve(&s, "one") else {
            panic!()
        };
        s.settle(
            &a.key,
            Some(30),
            Some(serde_json::json!({"private":"full evidence"})),
            "completed",
        )
        .unwrap();
        assert!(matches!(reserve(&s, "two"), Admission::Waiting(_)));
        assert!(s.snapshot().unwrap().attempts[0].result.is_none());
        assert!(s.attempt(&a.key).unwrap().unwrap().result.is_some());
    }
    #[test]
    fn explicit_pre_submission_rejection_releases_slot_without_erasing_identity() {
        let (_dir, s) = store();
        let Admission::Reserved(a) = reserve(&s, "rejected") else {
            panic!()
        };
        s.settle(&a.key, Some(0), None, "not_submitted").unwrap();
        assert_eq!(s.snapshot().unwrap().sol.used_calls, 0);
        assert!(matches!(reserve(&s, "rejected"), Admission::Existing(_)));
        assert!(matches!(reserve(&s, "another"), Admission::Reserved(_)));
    }
}
