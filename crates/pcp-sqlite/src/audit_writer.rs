//! Bounded persistence for derived access telemetry.
//!
//! Authoritative Page and authorization state never pass through this queue.

use std::{
    path::PathBuf,
    sync::{
        Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{Connection, params};
use tokio::{sync::oneshot, task};

use crate::store::open_connection;

const DEFAULT_QUEUE_CAPACITY: usize = 2_048;
const DEFAULT_BATCH_SIZE: usize = 512;
const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_MIN_COMMIT_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_SECURITY_FLUSH_WINDOW: Duration = Duration::from_millis(100);
const DEFAULT_ALLOWED_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const RETENTION_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETENTION_BACKLOG_INTERVAL: Duration = Duration::from_secs(60);
const RETENTION_DELETE_BATCH: usize = 5_000;

#[derive(Clone, Debug)]
pub(crate) struct AccessAuditPolicy {
    queue_capacity: usize,
    batch_size: usize,
    flush_interval: Duration,
    min_commit_interval: Duration,
    security_flush_window: Duration,
    allowed_retention: Duration,
}

impl Default for AccessAuditPolicy {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            batch_size: DEFAULT_BATCH_SIZE,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
            min_commit_interval: DEFAULT_MIN_COMMIT_INTERVAL,
            security_flush_window: DEFAULT_SECURITY_FLUSH_WINDOW,
            allowed_retention: DEFAULT_ALLOWED_RETENTION,
        }
    }
}

#[cfg(test)]
impl AccessAuditPolicy {
    pub(crate) fn for_test(flush_interval: Duration, allowed_retention: Duration) -> Self {
        Self {
            queue_capacity: 16,
            batch_size: 8,
            flush_interval,
            min_commit_interval: Duration::ZERO,
            security_flush_window: Duration::ZERO,
            allowed_retention,
        }
    }

    pub(crate) fn with_overload_limits(
        mut self,
        batch_size: usize,
        min_commit_interval: Duration,
        security_flush_window: Duration,
    ) -> Self {
        self.batch_size = batch_size;
        self.min_commit_interval = min_commit_interval;
        self.security_flush_window = security_flush_window;
        self
    }
}

#[derive(Debug)]
pub(crate) struct AccessAuditRecord {
    pub occurred_at: String,
    pub principal_json: String,
    pub session_id: String,
    pub operation: String,
    pub scopes_json: String,
    pub decision: String,
    pub detail: Option<String>,
    pub telemetry_json: Option<String>,
}

enum Command {
    Append(AccessAuditRecord),
    AppendDurable(AccessAuditRecord, oneshot::Sender<Result<(), String>>),
    Flush(oneshot::Sender<Result<(), String>>),
}

pub(crate) struct AccessAuditWriter {
    sender: Option<SyncSender<Command>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl AccessAuditWriter {
    pub(crate) fn start(path: PathBuf, policy: AccessAuditPolicy) -> Result<Self> {
        anyhow::ensure!(
            policy.queue_capacity > 0,
            "audit queue capacity must be positive"
        );
        anyhow::ensure!(policy.batch_size > 0, "audit batch size must be positive");
        anyhow::ensure!(
            !policy.flush_interval.is_zero(),
            "audit flush interval must be positive"
        );
        let (sender, receiver) = mpsc::sync_channel(policy.queue_capacity);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("pcp-access-audit".to_owned())
            .spawn(move || {
                let mut connection = match open_connection(&path) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = startup_sender.send(Err(error));
                        return;
                    }
                };
                let initial_pruned =
                    match prune_allowed_events(&connection, policy.allowed_retention) {
                        Ok(pruned) => pruned,
                        Err(error) => {
                            let _ = startup_sender.send(Err(error));
                            return;
                        }
                    };
                if startup_sender.send(Ok(())).is_err() {
                    return;
                }
                run_worker(&mut connection, receiver, policy, initial_pruned);
            })
            .context("start PCP access audit writer")?;
        match startup_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                sender: Some(sender),
                worker: Mutex::new(Some(worker)),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error).context("initialize PCP access audit writer")
            }
            Err(error) => {
                let _ = worker.join();
                Err(error).context("receive PCP access audit writer startup")
            }
        }
    }

    pub(crate) async fn enqueue(&self, record: AccessAuditRecord) -> Result<()> {
        self.send(Command::Append(record)).await
    }

    pub(crate) async fn append_durable(&self, record: AccessAuditRecord) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.send(Command::AppendDurable(record, reply)).await?;
        response
            .await
            .context("receive durable PCP access audit result")?
            .map_err(anyhow::Error::msg)
    }

    pub(crate) async fn flush(&self) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Flush(reply)).await?;
        response
            .await
            .context("receive PCP access audit flush result")?
            .map_err(anyhow::Error::msg)
    }

    async fn send(&self, command: Command) -> Result<()> {
        let sender = self
            .sender
            .as_ref()
            .context("PCP access audit writer is closed")?
            .clone();
        match sender.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Disconnected(_)) => {
                anyhow::bail!("PCP access audit writer stopped unexpectedly")
            }
            Err(TrySendError::Full(command)) => {
                task::spawn_blocking(move || sender.send(command).map_err(|_| ()))
                    .await
                    .context("join PCP access audit queue backpressure")?
                    .map_err(|()| anyhow::anyhow!("PCP access audit writer stopped unexpectedly"))
            }
        }
    }
}

impl Drop for AccessAuditWriter {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.lock().ok().and_then(|mut worker| worker.take())
            && worker.join().is_err()
        {
            eprintln!("PCP access audit writer panicked during shutdown");
        }
    }
}

fn run_worker(
    connection: &mut Connection,
    receiver: Receiver<Command>,
    policy: AccessAuditPolicy,
    initial_pruned: usize,
) {
    let mut pending = Vec::with_capacity(policy.batch_size);
    let mut pending_since = None;
    let mut last_commit = None;
    let mut next_retention_check = Instant::now()
        + if initial_pruned == RETENTION_DELETE_BATCH {
            RETENTION_BACKLOG_INTERVAL
        } else {
            RETENTION_CHECK_INTERVAL
        };
    loop {
        if pending.len() >= policy.batch_size {
            thread::sleep(commit_rate_limit_remaining(
                last_commit,
                policy.min_commit_interval,
            ));
            match flush_pending(connection, &mut pending) {
                Ok(committed) => {
                    if committed > 0 {
                        last_commit = Some(Instant::now());
                    }
                    pending_since = None;
                    maybe_prune(connection, &policy, &mut next_retention_check);
                }
                Err(error) => {
                    eprintln!("PCP access audit batch flush failed: {error:#}");
                    pending_since = Some(Instant::now());
                    thread::sleep(policy.flush_interval);
                    continue;
                }
            }
        }
        let wait = automatic_wait(pending_since, last_commit, &policy, next_retention_check);
        match receiver.recv_timeout(wait) {
            Ok(Command::Append(record)) => {
                if pending.is_empty() {
                    pending_since = Some(Instant::now());
                }
                pending.push(record);
            }
            Ok(Command::AppendDurable(record, reply)) => {
                if pending.is_empty() {
                    pending_since = Some(Instant::now());
                }
                pending.push(record);
                let mut replies = vec![reply];
                let deadline = Instant::now() + policy.security_flush_window;
                let mut disconnected = false;
                let mut explicit_barrier = false;
                while pending.len() < policy.batch_size {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    if wait.is_zero() {
                        break;
                    }
                    match receiver.recv_timeout(wait) {
                        Ok(Command::Append(record)) => pending.push(record),
                        Ok(Command::AppendDurable(record, reply)) => {
                            pending.push(record);
                            replies.push(reply);
                        }
                        Ok(Command::Flush(reply)) => {
                            replies.push(reply);
                            explicit_barrier = true;
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                if !explicit_barrier && !disconnected {
                    thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
                let result = flush_pending(connection, &mut pending);
                if let Ok(committed) = result.as_ref() {
                    if *committed > 0 {
                        last_commit = Some(Instant::now());
                    }
                    pending_since = None;
                    maybe_prune(connection, &policy, &mut next_retention_check);
                } else if pending_since.is_none() {
                    pending_since = Some(Instant::now());
                }
                send_flush_result(replies, &result);
                if disconnected {
                    return;
                }
            }
            Ok(Command::Flush(reply)) => {
                let result = flush_pending(connection, &mut pending);
                if let Ok(committed) = result.as_ref() {
                    if *committed > 0 {
                        last_commit = Some(Instant::now());
                    }
                    pending_since = None;
                    maybe_prune(connection, &policy, &mut next_retention_check);
                }
                send_flush_result(vec![reply], &result);
            }
            Err(RecvTimeoutError::Timeout) => {
                maybe_prune(connection, &policy, &mut next_retention_check);
                if pending_since.is_some_and(|started: Instant| {
                    started.elapsed() >= policy.flush_interval
                        && commit_rate_limit_remaining(last_commit, policy.min_commit_interval)
                            .is_zero()
                }) {
                    match flush_pending(connection, &mut pending) {
                        Ok(committed) => {
                            if committed > 0 {
                                last_commit = Some(Instant::now());
                            }
                            pending_since = None;
                        }
                        Err(error) => {
                            eprintln!("PCP access audit batch flush failed: {error:#}");
                            pending_since = Some(Instant::now());
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Err(error) = flush_pending(connection, &mut pending) {
                    eprintln!("PCP access audit shutdown flush failed: {error:#}");
                }
                return;
            }
        }
    }
}

fn automatic_wait(
    pending_since: Option<Instant>,
    last_commit: Option<Instant>,
    policy: &AccessAuditPolicy,
    next_retention_check: Instant,
) -> Duration {
    let retention_wait = next_retention_check.saturating_duration_since(Instant::now());
    let Some(pending_since) = pending_since else {
        return policy.flush_interval.min(retention_wait);
    };
    let age_wait = policy
        .flush_interval
        .saturating_sub(pending_since.elapsed());
    let commit_wait = commit_rate_limit_remaining(last_commit, policy.min_commit_interval);
    age_wait.max(commit_wait).min(retention_wait)
}

fn commit_rate_limit_remaining(
    last_commit: Option<Instant>,
    min_commit_interval: Duration,
) -> Duration {
    last_commit
        .map(|committed| min_commit_interval.saturating_sub(committed.elapsed()))
        .unwrap_or(Duration::ZERO)
}

fn send_flush_result(replies: Vec<oneshot::Sender<Result<(), String>>>, result: &Result<usize>) {
    let error = result.as_ref().err().map(|error| format!("{error:#}"));
    for reply in replies {
        let _ = reply.send(match error.as_ref() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        });
    }
}

fn flush_pending(
    connection: &mut Connection,
    pending: &mut Vec<AccessAuditRecord>,
) -> Result<usize> {
    if pending.is_empty() {
        return Ok(0);
    }
    let committed = pending.len();
    let transaction = connection
        .transaction()
        .context("start PCP access audit batch")?;
    {
        let mut statement = transaction
            .prepare_cached(
                "
                INSERT INTO pcp_access_log (
                    event_id, occurred_at, principal_json, session_id,
                    operation, scopes_json, decision, detail, telemetry_json
                ) VALUES (
                    'acc_' || lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
                )
                ",
            )
            .context("prepare PCP access audit batch")?;
        for record in pending.iter() {
            statement
                .execute(params![
                    record.occurred_at,
                    record.principal_json,
                    record.session_id,
                    record.operation,
                    record.scopes_json,
                    record.decision,
                    record.detail,
                    record.telemetry_json,
                ])
                .context("record PCP access event")?;
        }
    }
    transaction
        .commit()
        .context("commit PCP access audit batch")?;
    pending.clear();
    Ok(committed)
}

fn maybe_prune(
    connection: &Connection,
    policy: &AccessAuditPolicy,
    next_retention_check: &mut Instant,
) {
    if Instant::now() < *next_retention_check {
        return;
    }
    match prune_allowed_events(connection, policy.allowed_retention) {
        Ok(pruned) => {
            *next_retention_check = Instant::now()
                + if pruned == RETENTION_DELETE_BATCH {
                    RETENTION_BACKLOG_INTERVAL
                } else {
                    RETENTION_CHECK_INTERVAL
                };
        }
        Err(error) => {
            eprintln!("PCP access audit retention failed: {error:#}");
            *next_retention_check = Instant::now() + RETENTION_BACKLOG_INTERVAL;
        }
    }
}

fn prune_allowed_events(connection: &Connection, retention: Duration) -> Result<usize> {
    prune_allowed_events_with_limit(connection, retention, RETENTION_DELETE_BATCH)
}

fn prune_allowed_events_with_limit(
    connection: &Connection,
    retention: Duration,
    delete_batch: usize,
) -> Result<usize> {
    let delete_batch = i64::try_from(delete_batch).context("bound access audit delete batch")?;
    let retention =
        ChronoDuration::from_std(retention).context("convert access audit retention")?;
    let cutoff = (Utc::now() - retention).to_rfc3339_opts(SecondsFormat::Millis, true);
    connection
        .execute(
            "
            DELETE FROM pcp_access_log
            WHERE rowid IN (
                SELECT rowid
                FROM pcp_access_log
                WHERE decision = 'allowed' AND occurred_at < ?1
                ORDER BY occurred_at ASC, event_id ASC
                LIMIT ?2
            )
            ",
            params![cutoff, delete_batch],
        )
        .context("prune expired allowed PCP access events")
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
    use pcp_core::{AccessDecision, AccessPrincipal, AccessPrincipalType, AccessSession};
    use rusqlite::{Connection, params};

    use super::{AccessAuditPolicy, prune_allowed_events_with_limit};
    use crate::SqlitePcpStore;

    #[tokio::test]
    async fn allowed_events_wait_for_a_batch_flush() {
        let store = open_store(
            "batched",
            AccessAuditPolicy::for_test(Duration::from_secs(10), Duration::from_secs(86_400)),
        )
        .await;
        store
            .record_access(
                &access(),
                "read_pages",
                &["project:audit".to_owned()],
                &AccessDecision::Allowed,
                None,
                None,
            )
            .await
            .expect("enqueue allowed access event");

        assert_eq!(row_count(&store).await, 0);
        store
            .flush_access_audit()
            .await
            .expect("flush allowed access event");
        assert_eq!(row_count(&store).await, 1);
    }

    #[tokio::test]
    async fn denied_events_are_durable_before_returning() {
        let store = open_store(
            "denied",
            AccessAuditPolicy::for_test(Duration::from_secs(10), Duration::from_secs(86_400)),
        )
        .await;
        store
            .record_access(
                &access(),
                "write_page",
                &["project:audit".to_owned()],
                &AccessDecision::Denied,
                Some("authorization denied"),
                None,
            )
            .await
            .expect("record denied access event");

        assert_eq!(row_count(&store).await, 1);
    }

    #[tokio::test]
    async fn security_events_share_a_short_durable_window() {
        let store = open_store(
            "security-window",
            AccessAuditPolicy::for_test(Duration::from_secs(10), Duration::from_secs(86_400))
                .with_overload_limits(8, Duration::from_millis(100), Duration::from_millis(40)),
        )
        .await;
        let access = access();
        let scopes = ["project:audit".to_owned()];
        let started = Instant::now();
        let (denied, failed) = tokio::join!(
            store.record_access(
                &access,
                "write_page",
                &scopes,
                &AccessDecision::Denied,
                Some("authorization denied"),
                None,
            ),
            store.record_access(
                &access,
                "search_pages",
                &scopes,
                &AccessDecision::Failed,
                Some("operation failed"),
                None,
            ),
        );

        denied.expect("record denied access event");
        failed.expect("record failed access event");
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert_eq!(row_count(&store).await, 2);
    }

    #[tokio::test]
    async fn allowed_events_flush_after_the_bounded_interval() {
        let store = open_store(
            "interval",
            AccessAuditPolicy::for_test(Duration::from_millis(25), Duration::from_secs(86_400)),
        )
        .await;
        store
            .record_access(
                &access(),
                "search_pages",
                &["project:audit".to_owned()],
                &AccessDecision::Allowed,
                None,
                None,
            )
            .await
            .expect("enqueue allowed access event");

        tokio::time::sleep(Duration::from_millis(75)).await;
        assert_eq!(row_count(&store).await, 1);
    }

    #[tokio::test]
    async fn allowed_batch_threshold_respects_the_commit_rate_limit() {
        let store = open_store(
            "rate-limit",
            AccessAuditPolicy::for_test(Duration::from_secs(10), Duration::from_secs(86_400))
                .with_overload_limits(2, Duration::from_millis(100), Duration::ZERO),
        )
        .await;
        store
            .record_access(
                &access(),
                "read_pages",
                &["project:audit".to_owned()],
                &AccessDecision::Allowed,
                None,
                None,
            )
            .await
            .expect("enqueue initial allowed access event");
        store
            .flush_access_audit()
            .await
            .expect("establish initial audit commit");
        for operation in ["search_pages", "browse_index"] {
            store
                .record_access(
                    &access(),
                    operation,
                    &["project:audit".to_owned()],
                    &AccessDecision::Allowed,
                    None,
                    None,
                )
                .await
                .expect("enqueue rate-limited allowed event");
        }

        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(row_count(&store).await, 1);
        tokio::time::sleep(Duration::from_millis(125)).await;
        assert_eq!(row_count(&store).await, 3);
    }

    #[tokio::test]
    async fn retention_prunes_only_expired_allowed_events() {
        let path = temporary_database("retention");
        let initial = SqlitePcpStore::open(path.clone())
            .await
            .expect("initialize audit retention store");
        drop(initial);

        let old =
            (Utc::now() - ChronoDuration::days(2)).to_rfc3339_opts(SecondsFormat::Millis, true);
        let connection = Connection::open(&path).expect("open retention fixture");
        for (suffix, decision) in [
            ("allowed_a", "allowed"),
            ("allowed_b", "allowed"),
            ("denied", "denied"),
            ("failed", "failed"),
        ] {
            connection
                .execute(
                    "
                    INSERT INTO pcp_access_log (
                        event_id, occurred_at, principal_json, session_id,
                        operation, scopes_json, decision
                    ) VALUES (?1, ?2, ?3, 'session:audit', 'fixture', '[\"project:audit\"]', ?4)
                    ",
                    params![
                        format!("acc_old_{suffix}"),
                        old,
                        serde_json::to_string(&access().principal).expect("encode principal"),
                        decision,
                    ],
                )
                .expect("seed expired access event");
        }
        assert_eq!(
            prune_allowed_events_with_limit(&connection, Duration::from_secs(86_400), 1,)
                .expect("prune one retention batch"),
            1
        );
        let remaining_allowed = connection
            .query_row(
                "SELECT count(*) FROM pcp_access_log WHERE decision = 'allowed'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count allowed events after bounded prune");
        assert_eq!(remaining_allowed, 1);
        drop(connection);

        let store = SqlitePcpStore::open_with_access_audit_policy(
            path,
            AccessAuditPolicy::for_test(Duration::from_secs(10), Duration::from_secs(86_400)),
        )
        .await
        .expect("reopen audit retention store");
        let decisions = store
            .run("retention verification", |connection| {
                let mut statement =
                    connection.prepare("SELECT decision FROM pcp_access_log ORDER BY decision")?;
                Ok(statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .expect("read retained decisions");

        assert_eq!(decisions, vec!["denied", "failed"]);
    }

    async fn open_store(name: &str, policy: AccessAuditPolicy) -> SqlitePcpStore {
        SqlitePcpStore::open_with_access_audit_policy(temporary_database(name), policy)
            .await
            .expect("open audit test store")
    }

    fn access() -> AccessSession {
        AccessSession::full_control(
            AccessPrincipal {
                principal_id: "client:audit-test".to_owned(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "session:audit-test",
            ["project:audit".to_owned()],
        )
    }

    async fn row_count(store: &SqlitePcpStore) -> i64 {
        store
            .run("audit row count", |connection| {
                Ok(connection
                    .query_row("SELECT count(*) FROM pcp_access_log", [], |row| row.get(0))?)
            })
            .await
            .expect("count access audit rows")
    }

    fn temporary_database(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pcp-audit-writer-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create audit test root");
        root.join("pcp.sqlite3")
    }
}
