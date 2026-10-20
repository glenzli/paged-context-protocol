//! Cached, read-only discovery of the maintenance application's authorized models.
//! Uses public Discovery/Core negotiation; never reads an operator credential.
use anyhow::{Context, Result, ensure};
use infer_runtime_client::{
    CAPABILITY_CONTRACT_HEADER, CONSUMER_CORE, CONSUMER_CORE_HEADER, Client, DiscoveryResolver,
};
use pcp_runtime::{MaintenanceWorkerConfig, RuntimeConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const IDENTITY: &str = "infer.model-catalog@20261016.1";
const SCHEMA_SHA256: &str = "bcf9c70f6a708731e9655f135052b0dfe8cbf0b828c78b9c001e89c2c8f6cc32";
const MAX_BYTES: usize = 1024 * 1024;
const SUCCESS_TTL: Duration = Duration::from_secs(60);
const ERROR_TTL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Model {
    pub deployment_id: String,
    pub supported_efforts: Vec<String>,
    pub intents: BTreeMap<String, String>,
    pub availability: String,
    pub observation_stale: bool,
    pub provider_access_class: String,
    pub placement: String,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Snapshot {
    pub models: Vec<Model>,
    pub available: bool,
    pub stale: bool,
    pub fetched_at: Option<String>,
    pub error: Option<&'static str>,
}
#[derive(Default)]
struct Cache {
    key: String,
    attempted: Option<Instant>,
    snapshot: Snapshot,
}
#[derive(Default)]
pub(super) struct ModelCatalog {
    cache: Mutex<Cache>,
}

impl ModelCatalog {
    pub(super) async fn snapshot(&self, config: Option<&Path>, refresh: bool) -> Snapshot {
        let source = (|| -> Result<_> {
            let config = RuntimeConfig::load(config.context("missing configuration")?)?;
            let MaintenanceWorkerConfig::InferRuntime {
                credential_file, ..
            } = config.maintenance.context("missing maintenance")?.worker
            else {
                anyhow::bail!("not an Infer worker")
            };
            let token = read_token(&credential_file)?;
            let resolver = DiscoveryResolver::local();
            let endpoint = resolver.resolve()?;
            let key = format!(
                "{}:{}:{:x}",
                endpoint.endpoint,
                endpoint.generation,
                Sha256::digest(token.as_bytes())
            );
            Ok((resolver, endpoint, token, key))
        })();
        let Ok((resolver, endpoint, token, key)) = source else {
            // A changed or revoked credential must not reuse an old authorization view.
            *self.cache.lock().await = Cache::default();
            return Snapshot {
                error: Some("unavailable"),
                ..Default::default()
            };
        };
        let mut cache = self.cache.lock().await;
        if cache.key != key {
            *cache = Cache {
                key,
                ..Default::default()
            };
        }
        let ttl = if cache.snapshot.available && !cache.snapshot.stale {
            SUCCESS_TTL
        } else {
            ERROR_TTL
        };
        if cache
            .attempted
            .is_some_and(|at| at.elapsed() < ttl && (!refresh || at.elapsed() < ERROR_TTL))
        {
            return cache.snapshot.clone();
        }
        cache.attempted = Some(Instant::now());
        match tokio::time::timeout(Duration::from_secs(5), fetch(resolver, endpoint, &token)).await
        {
            Ok(Ok(models)) => {
                cache.snapshot = Snapshot {
                    models,
                    available: true,
                    stale: false,
                    fetched_at: Some(chrono::Utc::now().to_rfc3339()),
                    error: None,
                }
            }
            _ => {
                cache.snapshot.stale = cache.snapshot.available;
                cache.snapshot.error = Some("unavailable");
            }
        }
        cache.snapshot.clone()
    }
}

fn read_token(path: &Path) -> Result<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0,
        "invalid credential permissions"
    );
    ensure!(metadata.len() <= 4096, "oversize credential");
    let mut token = String::new();
    Read::by_ref(&mut file)
        .take(4097)
        .read_to_string(&mut token)?;
    ensure!(
        token.len() <= 4096
            && !token.trim().is_empty()
            && !token.trim().contains(char::is_whitespace),
        "invalid credential"
    );
    Ok(token.trim().to_owned())
}

async fn bounded(mut response: reqwest::Response) -> Result<Vec<u8>> {
    ensure!(response.status().is_success(), "model catalog unavailable");
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MAX_BYTES as u64),
        "oversize model catalog"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "oversize model catalog"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn fetch(
    resolver: DiscoveryResolver,
    endpoint: infer_runtime_client::ResolvedEndpoint,
    token: &str,
) -> Result<Vec<Model>> {
    // The pinned SDK still supplies the shared Discovery and Core negotiation.
    // This additive capability has an independent wire/schema identity.
    let client = Client::with_discovery(resolver.clone()).build()?;
    client.contract().await?;
    let catalog = client.capabilities().await?;
    let capability = catalog
        .capabilities
        .iter()
        .find(|c| c.id == "infer.model-catalog" && c.schema_version == "20261016.1")
        .context("model catalog capability unavailable")?;
    let schema_path = "/infer/v1/capability-schemas/infer.model-catalog/20261016.1/openapi.json";
    ensure!(
        capability.schema.url == schema_path
            && capability.schema.sha256 == SCHEMA_SHA256
            && capability
                .routes
                .iter()
                .any(|r| r.method == "GET" && r.path == "/infer/v1/model-catalog"),
        "unsupported model catalog contract"
    );
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let schema = bounded(
        http.get(format!("{}{schema_path}", endpoint.endpoint))
            .send()
            .await?,
    )
    .await?;
    ensure!(
        format!("{:x}", Sha256::digest(&schema)) == SCHEMA_SHA256,
        "model catalog schema changed"
    );
    ensure!(resolver.resolve()? == endpoint, "Infer generation changed");
    let bytes = bounded(
        http.get(format!("{}/infer/v1/model-catalog", endpoint.endpoint))
            .header(CONSUMER_CORE_HEADER, CONSUMER_CORE)
            .header(CAPABILITY_CONTRACT_HEADER, IDENTITY)
            .bearer_auth(token)
            .send()
            .await?,
    )
    .await?;
    #[derive(Deserialize)]
    struct Reply {
        deployments: Vec<Model>,
    }
    maintenance_models(serde_json::from_slice::<Reply>(&bytes)?.deployments)
}

fn maintenance_models(models: Vec<Model>) -> Result<Vec<Model>> {
    ensure!(models.len() <= 1024, "oversize model catalog");
    Ok(models
        .into_iter()
        .filter(|m| m.provider_access_class == "subscription" && m.placement == "cloud")
        .filter(|m| {
            ["language.respond", "reasoning.solve"]
                .iter()
                .any(|intent| {
                    m.intents
                        .get(*intent)
                        .is_some_and(|level| matches!(level.as_str(), "advanced" | "exceptional"))
                })
        })
        .filter(|m| {
            !m.deployment_id.is_empty()
                && m.deployment_id.len() <= 256
                && m.supported_efforts.len() <= 7
                && m.supported_efforts.iter().all(|e| {
                    matches!(
                        e.as_str(),
                        "none" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
                    )
                })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_credential_requires_an_owner_only_regular_file() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!(
            "pcp-catalog-token-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let token = root.join("credential");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&token)
            .unwrap();
        file.write_all(b"test-credential\n").unwrap();
        drop(file);
        assert_eq!(read_token(&token).unwrap(), "test-credential");
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token(&token).is_err());
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = root.join("symlink");
        std::os::unix::fs::symlink(&token, &link).unwrap();
        assert!(read_token(&link).is_err());
        std::fs::write(&token, vec![b'x'; 4097]).unwrap();
        assert!(read_token(&token).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn maintenance_catalog_accepts_new_versions_and_filters_wrong_execution_boundaries() {
        let model = Model {
            deployment_id: "codex_gpt_6_1_sol".into(),
            supported_efforts: vec!["high".into(), "ultra".into()],
            intents: BTreeMap::from([("reasoning.solve".into(), "exceptional".into())]),
            availability: "available".into(),
            observation_stale: false,
            provider_access_class: "subscription".into(),
            placement: "cloud".into(),
        };
        let mut local = model.clone();
        local.placement = "local".into();
        let mut basic = model.clone();
        basic
            .intents
            .insert("reasoning.solve".into(), "foundational".into());
        let mut malformed = model.clone();
        malformed.supported_efforts.push("future_effort".into());
        let result = maintenance_models(vec![local, basic, malformed, model]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].deployment_id, "codex_gpt_6_1_sol");
        assert_eq!(result[0].supported_efforts, ["high", "ultra"]);
    }
}
