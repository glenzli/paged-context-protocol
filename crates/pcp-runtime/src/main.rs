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
    CommandSemanticWorker, ObserverConfig, ObserverService, RuntimeConfig, RuntimeMaintainer,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    let config_path = match arguments.next().as_deref() {
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

async fn run_broker(config_path: PathBuf) -> Result<()> {
    let config = RuntimeConfig::load(&config_path)?;
    let store = Arc::new(
        SqlitePcpStore::open(config.store_path.clone())
            .await
            .with_context(|| format!("open PCP Store {}", config.store_path.display()))?,
    );
    let owner_id = store.owner_id().to_owned();
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
                    endpoint.access_session(&owner_id, index)?,
                ),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let maintenance_task = if let Some(maintenance) =
        config.maintenance.filter(|maintenance| maintenance.enabled)
    {
        let client =
            EmbeddedPcpClient::shared(Arc::clone(&store), maintenance.access_session(&owner_id));
        let worker = Arc::new(CommandSemanticWorker::new(&maintenance.worker));
        let maintainer = RuntimeMaintainer::load(client, worker, maintenance).await?;
        Some(tokio::spawn(maintainer.run_forever()))
    } else {
        None
    };
    let mut observer =
        ObserverService::start(ObserverConfig::from_env(&owner_id)?, Arc::clone(&store)).await?;
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
    let owner_id = store.owner_id().to_owned();
    let client = EmbeddedPcpClient::shared(Arc::clone(&store), access);
    let mut observer =
        ObserverService::start(ObserverConfig::from_env(&owner_id)?, Arc::clone(&store)).await?;
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
        "pcp-runtime [--config <runtime.toml>]\n\nUse --config or PCP_RUNTIME_CONFIG for a multi-endpoint broker. Without a config, one identity-bound endpoint is read from PCP_STORE_PATH, PCP_RUNTIME_SOCKET, PCP_CLIENT_ID, PCP_CLIENT_TYPE, PCP_ACCESS_MODE, and PCP_ALLOWED_SCOPES. PCP observer discovery uses the platform Infra Protocol runtime root or the final INFRA_PROTOCOL_RUNTIME_DIR override; set PCP_OBSERVER_ENABLED=0 to disable it."
    );
}
