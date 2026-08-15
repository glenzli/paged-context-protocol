use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::{AccessMode, EmbeddedPcpClient};
use pcp_core::{AccessPrincipal, AccessPrincipalType};
use pcp_rpc::{RuntimeEndpoint, serve_unix, serve_unix_endpoints};
use pcp_runtime::{
    EnrollmentConfig, MaintenanceMode, MaintenanceRunAudit, ObserverConfig, ObserverService,
    RuntimeConfig, RuntimeMaintainer, build_semantic_worker, persist_audit,
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
        action == "run-once",
        "unsupported PCP maintenance action: {action}"
    );
    let mut config_path = None;
    let mut mode = None;
    let mut max_jobs = None;
    let mut reason = None;
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
            other => anyhow::bail!("unknown PCP maintenance run-once option: {other}"),
        }
    }
    let config_path = config_path.context("maintenance run-once requires --config")?;
    anyhow::ensure!(
        mode.as_deref() == Some("observe"),
        "maintenance run-once requires --mode observe"
    );
    anyhow::ensure!(
        max_jobs == Some(1),
        "maintenance run-once requires --max-jobs 1"
    );
    let reason = reason.context("maintenance run-once requires --reason")?;
    anyhow::ensure!(
        !reason.trim().is_empty() && reason.len() <= 120 && !reason.contains(['\n', '\r']),
        "maintenance run-once reason must contain 1-120 non-line-break characters"
    );
    run_operator_maintenance_once(config_path, reason).await
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
    let endpoints = config
        .endpoints
        .iter()
        .enumerate()
        .map(|(index, endpoint)| {
            Ok(RuntimeEndpoint {
                socket_path: endpoint.socket_path.clone(),
                client: EmbeddedPcpClient::shared(
                    Arc::clone(&store),
                    endpoint.access_session(&identity_id, index)?,
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
        let maintainer = RuntimeMaintainer::load(client, worker, maintenance).await?;
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
    let mut observer =
        ObserverService::start(observer_config, enrollment_config, Arc::clone(&store)).await?;
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
        "pcp-runtime [--config <runtime.toml>]\npcp-runtime maintenance run-once --config <runtime.toml> --mode observe --max-jobs 1 --reason <reason>\n\nUse --config or PCP_RUNTIME_CONFIG for a multi-endpoint broker. The maintenance command is a same-user local operator control: it permits only one observe-mode job, writes a redacted audit record, and does not alter normal scheduler cadence. Without a config, one identity-bound endpoint is read from PCP_STORE_PATH, PCP_RUNTIME_SOCKET, PCP_CLIENT_ID, PCP_CLIENT_TYPE, PCP_ACCESS_MODE, and PCP_ALLOWED_SCOPES. PCP observer and enrollment discovery use the platform Infra Protocol runtime root or the final INFRA_PROTOCOL_RUNTIME_DIR override. PCP_OBSERVER_ENABLED and PCP_ENROLLMENT_ENABLED disable their respective offers."
    );
}

fn print_maintenance_help() {
    println!(
        "pcp-runtime maintenance run-once --config <runtime.toml> --mode observe --max-jobs 1 --reason <reason>"
    );
}
