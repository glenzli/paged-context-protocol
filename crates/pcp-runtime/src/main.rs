use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::{AccessMode, EmbeddedPcpClient, PcpApi};
use pcp_core::{AccessPrincipal, AccessPrincipalType};
use pcp_rpc::{RuntimeEndpoint, serve_unix, serve_unix_endpoints};
use pcp_runtime::{
    EnrollmentConfig, MaintenanceMode, MaintenanceRunAudit, ObserverConfig, ObserverService,
    QueryRuntime, RuntimeConfig, RuntimeMaintainer, build_semantic_worker, persist_audit,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    let config_path = match arguments.next().as_deref() {
        Some("maintenance") => return run_maintenance_command(arguments).await,
        Some("--config") => Some(PathBuf::from(
            arguments
                .next()
                .context("pcp-runtime --config requires a TOML path")?,
        )),
        Some("--help" | "-h") => {
            print_help();
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown pcp-runtime argument: {other}"),
        None => env::var_os("PCP_RUNTIME_CONFIG").map(PathBuf::from),
    };
    anyhow::ensure!(
        arguments.next().is_none(),
        "pcp-runtime accepts only one --config path"
    );
    if let Some(config_path) = config_path {
        return run_broker(config_path).await;
    }
    run_single_endpoint().await
}

async fn run_maintenance_command(mut arguments: impl Iterator<Item = String>) -> Result<()> {
    let action = arguments
        .next()
        .context("pcp-runtime maintenance requires an action")?;
    if action == "--help" || action == "-h" {
        print_maintenance_help();
        return Ok(());
    }
    anyhow::ensure!(
        matches!(action.as_str(), "run-once" | "run-batch"),
        "unsupported PCP maintenance action: {action}"
    );
    let mut config_path = None;
    let mut mode = None;
    let mut max_jobs = None;
    let mut reason = None;
    let mut confirm_identity = None;
    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .with_context(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--config" => set_option(&mut config_path, PathBuf::from(value), "--config")?,
            "--mode" => set_option(&mut mode, value, "--mode")?,
            "--max-jobs" => set_option(
                &mut max_jobs,
                value
                    .parse::<u32>()
                    .context("--max-jobs must be an unsigned integer")?,
                "--max-jobs",
            )?,
            "--reason" => set_option(&mut reason, value, "--reason")?,
            "--confirm-identity" => set_option(&mut confirm_identity, value, "--confirm-identity")?,
            other => anyhow::bail!("unknown PCP maintenance run-once option: {other}"),
        }
    }
    let config_path = config_path.context("maintenance run-once requires --config")?;
    let reason = reason.with_context(|| format!("maintenance {action} requires --reason"))?;
    anyhow::ensure!(
        !reason.trim().is_empty() && reason.len() <= 120 && !reason.contains(['\n', '\r']),
        "maintenance run-once reason must contain 1-120 non-line-break characters"
    );
    if action == "run-once" {
        anyhow::ensure!(
            mode.as_deref() == Some("observe"),
            "maintenance run-once requires --mode observe"
        );
        anyhow::ensure!(
            max_jobs == Some(1),
            "maintenance run-once requires --max-jobs 1"
        );
        anyhow::ensure!(
            confirm_identity.is_none(),
            "maintenance run-once does not accept --confirm-identity"
        );
        return run_operator_maintenance_once(config_path, reason).await;
    }

    let mode = match mode.as_deref() {
        Some("observe") => MaintenanceMode::Observe,
        Some("apply") => MaintenanceMode::Apply,
        _ => anyhow::bail!("maintenance run-batch requires --mode observe or apply"),
    };
    let max_jobs = max_jobs.context("maintenance run-batch requires --max-jobs")?;
    anyhow::ensure!(
        (1..=1_000).contains(&max_jobs),
        "maintenance run-batch --max-jobs must be between 1 and 1000"
    );
    let confirm_identity =
        confirm_identity.context("maintenance run-batch requires --confirm-identity")?;
    run_operator_maintenance_batch(config_path, reason, mode, max_jobs, confirm_identity).await
}

async fn run_operator_maintenance_once(config_path: PathBuf, reason: String) -> Result<()> {
    let mut config = RuntimeConfig::load(&config_path)?;
    let maintenance = config
        .maintenance
        .take()
        .context("PCP runtime config has no maintenance section")?;
    anyhow::ensure!(
        maintenance.enabled && maintenance.mode == MaintenanceMode::Observe,
        "operator maintenance run-once requires enabled observe maintenance"
    );
    let audit_path = maintenance
        .state_path
        .with_file_name("maintenance-audit.json");
    let store = Arc::new(
        SqlitePcpStore::open(config.store_path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", config.store_path.display()))?,
    );
    let identity_id = store.identity_id().to_owned();
    let store: Arc<dyn PcpStore> = store;
    let client =
        EmbeddedPcpClient::shared(Arc::clone(&store), maintenance.access_session(&identity_id));
    let audit = MaintenanceRunAudit::queued(reason);
    let worker = audit.worker(build_semantic_worker(&maintenance.worker)?);
    let mut maintainer =
        RuntimeMaintainer::load_operator_observe_once(client, worker, maintenance).await?;
    match maintainer.run_operator_observe_once().await {
        Ok(report) => {
            let record = audit.complete(report);
            persist_audit(&audit_path, record.clone()).await?;
            println!(
                "{}",
                serde_json::to_string(&record).context("encode maintenance result")?
            );
            Ok(())
        }
        Err(error) => {
            let record = audit.fail("scheduler");
            persist_audit(&audit_path, record.clone()).await?;
            eprintln!(
                "{}",
                serde_json::to_string(&record).context("encode maintenance result")?
            );
            Err(error).context("run PCP operator maintenance once")
        }
    }
}

async fn run_operator_maintenance_batch(
    config_path: PathBuf,
    reason: String,
    mode: MaintenanceMode,
    max_jobs: u32,
    confirm_identity: String,
) -> Result<()> {
    let mut config = RuntimeConfig::load(&config_path)?;
    let mut maintenance = config
        .maintenance
        .take()
        .context("PCP runtime config has no maintenance section")?;
    maintenance.enabled = true;
    maintenance.mode = mode;
    maintenance.validate()?;
    let audit_path = maintenance
        .state_path
        .with_file_name("maintenance-audit.json");
    let store = Arc::new(
        SqlitePcpStore::open(config.store_path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", config.store_path.display()))?,
    );
    let identity_id = store.identity_id().to_owned();
    anyhow::ensure!(
        confirm_identity == identity_id,
        "maintenance run-batch identity confirmation does not match the Store"
    );
    let store: Arc<dyn PcpStore> = store;
    let client =
        EmbeddedPcpClient::shared(Arc::clone(&store), maintenance.access_session(&identity_id));
    let mode_name = match mode {
        MaintenanceMode::Observe => "observe",
        MaintenanceMode::Apply => "apply",
    };
    let audit = MaintenanceRunAudit::queued_with_limits(mode_name, max_jobs, reason);
    let worker = audit.worker(build_semantic_worker(&maintenance.worker)?);
    let mut maintainer = RuntimeMaintainer::load(client, worker, maintenance).await?;
    let mut aggregate = pcp_runtime::MaintenanceCycleReport::default();

    while aggregate.worker_calls < max_jobs {
        let remaining = max_jobs.saturating_sub(aggregate.worker_calls);
        let report = match maintainer.run_once_with_job_limit(remaining).await {
            Ok(report) => report,
            Err(error) => {
                let record = audit.fail("batch");
                persist_audit(&audit_path, record.clone()).await?;
                eprintln!(
                    "{}",
                    serde_json::to_string(&record).context("encode maintenance result")?
                );
                return Err(error).context("run PCP operator maintenance batch");
            }
        };
        let worker_calls = report.worker_calls;
        merge_maintenance_report(&mut aggregate, report);
        if worker_calls == 0 || aggregate.worker_calls >= max_jobs {
            break;
        }
    }

    let record = audit.complete(aggregate);
    persist_audit(&audit_path, record.clone()).await?;
    println!(
        "{}",
        serde_json::to_string(&record).context("encode maintenance result")?
    );
    Ok(())
}

fn merge_maintenance_report(
    aggregate: &mut pcp_runtime::MaintenanceCycleReport,
    report: pcp_runtime::MaintenanceCycleReport,
) {
    aggregate.inspected_pages = aggregate.inspected_pages.max(report.inspected_pages);
    aggregate.worker_calls = aggregate.worker_calls.saturating_add(report.worker_calls);
    aggregate.summaries_written = aggregate
        .summaries_written
        .saturating_add(report.summaries_written);
    aggregate.summaries_proposed = aggregate
        .summaries_proposed
        .saturating_add(report.summaries_proposed);
    aggregate.packs_committed = aggregate
        .packs_committed
        .saturating_add(report.packs_committed);
    aggregate.packs_proposed = aggregate
        .packs_proposed
        .saturating_add(report.packs_proposed);
    aggregate.relations_committed = aggregate
        .relations_committed
        .saturating_add(report.relations_committed);
    aggregate.relations_proposed = aggregate
        .relations_proposed
        .saturating_add(report.relations_proposed);
    aggregate.retention_leases_written = aggregate
        .retention_leases_written
        .saturating_add(report.retention_leases_written);
    aggregate.retention_leases_proposed = aggregate
        .retention_leases_proposed
        .saturating_add(report.retention_leases_proposed);
    aggregate.deferred = aggregate.deferred.saturating_add(report.deferred);
}

fn set_option<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<()> {
    anyhow::ensure!(slot.is_none(), "{name} may be provided only once");
    *slot = Some(value);
    Ok(())
}

async fn run_broker(config_path: PathBuf) -> Result<()> {
    let config = RuntimeConfig::load(&config_path)?;
    let store_path = config.store_path.clone();
    let runtime_socket_hint = config.endpoints[0].socket_path.clone();
    let store = Arc::new(
        SqlitePcpStore::open(config.store_path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", config.store_path.display()))?,
    );
    let identity_id = store.identity_id().to_owned();
    let store: Arc<dyn PcpStore> = store;
    let query_service = Arc::new(QueryRuntime::from_config(
        Arc::clone(&store),
        config.semantic_search.clone(),
        config.intent_match.clone(),
    )?);
    let (successful_write_observer, maintenance_write_wake) = if config
        .maintenance
        .as_ref()
        .is_some_and(|value| value.enabled)
    {
        let (sender, receiver) = tokio::sync::watch::channel(0_u64);
        let observer: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            sender.send_modify(|generation| *generation = generation.wrapping_add(1));
        });
        (Some(observer), Some(receiver))
    } else {
        (None, None)
    };
    let endpoints = config
        .endpoints
        .iter()
        .enumerate()
        .map(|(index, endpoint)| {
            let mut client = EmbeddedPcpClient::new(
                Arc::clone(&store),
                endpoint.access_session(&identity_id, index)?,
            );
            if let Some(observer) = successful_write_observer.as_ref() {
                client = client.with_successful_write_observer(Arc::clone(observer));
            }
            Ok(RuntimeEndpoint {
                socket_path: endpoint.socket_path.clone(),
                client: Arc::new(client) as Arc<dyn PcpApi>,
                query_service: Some(
                    Arc::clone(&query_service) as Arc<dyn pcp_rpc::RuntimeQueryService>
                ),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let maintenance_task = if let Some(maintenance) =
        config.maintenance.filter(|maintenance| maintenance.enabled)
    {
        let client =
            EmbeddedPcpClient::shared(Arc::clone(&store), maintenance.access_session(&identity_id));
        let worker = build_semantic_worker(&maintenance.worker)?;
        let mut maintainer = RuntimeMaintainer::load(client, worker, maintenance).await?;
        if let Some(write_wake) = maintenance_write_wake {
            maintainer = maintainer.with_write_wakeup(write_wake);
        }
        Some(tokio::spawn(maintainer.run_forever()))
    } else {
        None
    };
    let observer_config = ObserverConfig::from_env(&identity_id)?;
    let enrollment_config = EnrollmentConfig::from_env(
        observer_config.runtime_root.clone(),
        store_path,
        runtime_socket_hint,
    )?;
    let mut observer = ObserverService::start_with_query(
        observer_config,
        enrollment_config,
        Arc::clone(&store),
        Some(query_service),
    )
    .await?;
    let result =
        supervise_runtime(tokio::spawn(serve_unix_endpoints(endpoints)), &mut observer).await;
    if let Some(task) = maintenance_task {
        task.abort();
        let _ = task.await;
    }
    result
}

async fn run_single_endpoint() -> Result<()> {
    let store_path = env::var_os("PCP_STORE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/context.sqlite3"));
    let socket_path = env::var_os("PCP_RUNTIME_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/pcp-runtime.sock"));
    let scopes = required_scopes()?;
    let principal_id =
        env::var("PCP_CLIENT_ID").context("PCP_CLIENT_ID must identify this runtime endpoint")?;
    anyhow::ensure!(
        !principal_id.trim().is_empty(),
        "PCP_CLIENT_ID must not be empty"
    );
    let access_mode = env::var("PCP_ACCESS_MODE")
        .unwrap_or_else(|_| "read".to_owned())
        .parse::<AccessMode>()?;
    let principal_type = env::var("PCP_CLIENT_TYPE")
        .ok()
        .map(|value| parse_principal_type(&value))
        .transpose()?
        .unwrap_or(AccessPrincipalType::Service);
    let allow_cross_scope_derivation = env::var("PCP_ALLOW_CROSS_SCOPE_DERIVATION")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let access = access_mode.session(
        AccessPrincipal {
            principal_id,
            principal_type,
            display_name: env::var("PCP_CLIENT_NAME").ok(),
        },
        env::var("PCP_SESSION_ID")
            .unwrap_or_else(|_| format!("pcp-runtime:{}:{started}", std::process::id())),
        scopes,
        allow_cross_scope_derivation,
    );
    let store: Arc<dyn PcpStore> = Arc::new(
        SqlitePcpStore::open(store_path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", store_path.display()))?,
    );
    let identity_id = store.identity_id().to_owned();
    let client = EmbeddedPcpClient::shared(Arc::clone(&store), access);
    let observer_config = ObserverConfig::from_env(&identity_id)?;
    let enrollment_config = EnrollmentConfig::from_env(
        observer_config.runtime_root.clone(),
        store_path,
        socket_path.clone(),
    )?;
    let mut observer =
        ObserverService::start(observer_config, enrollment_config, Arc::clone(&store)).await?;
    supervise_runtime(tokio::spawn(serve_unix(socket_path, client)), &mut observer).await
}

async fn supervise_runtime(
    mut runtime: tokio::task::JoinHandle<Result<()>>,
    observer: &mut Option<ObserverService>,
) -> Result<()> {
    let result = if let Some(observer) = observer.as_mut() {
        tokio::select! {
            result = &mut runtime => flatten_runtime_join(result),
            result = observer.wait() => match result {
                Ok(()) => Err(anyhow::anyhow!("PCP observer stopped unexpectedly")),
                Err(error) => Err(error).context("PCP observer stopped"),
            },
            signal = shutdown_signal() => signal,
        }
    } else {
        tokio::select! {
            result = &mut runtime => flatten_runtime_join(result),
            signal = shutdown_signal() => signal,
        }
    };

    runtime.abort();
    let _ = runtime.await;
    let observer_shutdown = if let Some(observer) = observer.as_mut() {
        observer.shutdown().await
    } else {
        Ok(())
    };
    result.and(observer_shutdown)
}

fn flatten_runtime_join(result: Result<Result<()>, tokio::task::JoinError>) -> Result<()> {
    result.context("join PCP runtime endpoints")?
}

async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install PCP SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("wait for PCP interrupt")?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

fn required_scopes() -> Result<Vec<String>> {
    let scopes = env::var("PCP_ALLOWED_SCOPES")
        .context("PCP_ALLOWED_SCOPES must explicitly name at least one Scope")?
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !scopes.is_empty(),
        "PCP_ALLOWED_SCOPES must explicitly name at least one Scope"
    );
    Ok(scopes)
}

fn parse_principal_type(value: &str) -> Result<AccessPrincipalType> {
    match value {
        "host" => Ok(AccessPrincipalType::Host),
        "model_client" => Ok(AccessPrincipalType::ModelClient),
        "cli" => Ok(AccessPrincipalType::Cli),
        "service" => Ok(AccessPrincipalType::Service),
        other => anyhow::bail!("unsupported PCP_CLIENT_TYPE: {other}"),
    }
}

fn print_help() {
    println!(
        "pcp-runtime [--config <runtime.toml>]\npcp-runtime maintenance run-once --config <runtime.toml> --mode observe --max-jobs 1 --reason <reason>\npcp-runtime maintenance run-batch --config <runtime.toml> --mode <observe|apply> --max-jobs <1..1000> --confirm-identity <idn_...> --reason <reason>\n\nUse --config or PCP_RUNTIME_CONFIG for a multi-endpoint broker. Maintenance commands are same-user local operator controls and write a redacted audit record. run-once permits only one observe-mode job and does not alter normal scheduler cadence. run-batch persists the maintenance ledger, stops at its worker-call budget or when no eligible work remains, and requires an exact Store identity confirmation. Without a config, one identity-bound endpoint is read from PCP_STORE_PATH, PCP_RUNTIME_SOCKET, PCP_CLIENT_ID, PCP_CLIENT_TYPE, PCP_ACCESS_MODE, and PCP_ALLOWED_SCOPES. PCP observer and enrollment discovery use the platform Infra Protocol runtime root or the final INFRA_PROTOCOL_RUNTIME_DIR override. PCP_OBSERVER_ENABLED and PCP_ENROLLMENT_ENABLED disable their respective offers."
    );
}

fn print_maintenance_help() {
    println!(
        "pcp-runtime maintenance run-once --config <runtime.toml> --mode observe --max-jobs 1 --reason <reason>\npcp-runtime maintenance run-batch --config <runtime.toml> --mode <observe|apply> --max-jobs <1..1000> --confirm-identity <idn_...> --reason <reason>"
    );
}
