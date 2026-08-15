use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pcp_client::AccessMode;
use pcp_core::{AccessPrincipal, AccessPrincipalType, AccessSession};
use serde::Deserialize;

use crate::MaintenanceConfig;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub store_path: PathBuf,
    pub endpoints: Vec<RuntimeEndpointConfig>,
    pub maintenance: Option<MaintenanceConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEndpointConfig {
    pub socket_path: PathBuf,
    pub client_id: String,
    #[serde(default = "default_client_type")]
    pub client_type: String,
    pub client_name: Option<String>,
    #[serde(default = "default_access_mode")]
    pub access_mode: String,
    pub allowed_scopes: Vec<String>,
    #[serde(default)]
    pub allow_cross_scope_derivation: bool,
    pub session_id: Option<String>,
}

impl RuntimeConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("read PCP runtime config {}", path.display()))?;
        let mut config: Self = toml::from_str(&source)
            .with_context(|| format!("parse PCP runtime config {}", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        config.resolve_paths(base);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.endpoints.is_empty(),
            "PCP runtime config requires at least one endpoint"
        );
        let mut sockets = HashSet::new();
        let mut principals = HashSet::new();
        for endpoint in &self.endpoints {
            anyhow::ensure!(
                !endpoint.client_id.trim().is_empty(),
                "PCP runtime endpoint client_id must not be empty"
            );
            anyhow::ensure!(
                !endpoint.allowed_scopes.is_empty(),
                "PCP runtime endpoint {} requires at least one allowed Scope",
                endpoint.client_id
            );
            anyhow::ensure!(
                endpoint
                    .allowed_scopes
                    .iter()
                    .all(|scope| !scope.trim().is_empty()),
                "PCP runtime endpoint {} contains an empty Scope",
                endpoint.client_id
            );
            endpoint.access_mode.parse::<AccessMode>()?;
            parse_principal_type(&endpoint.client_type)?;
            anyhow::ensure!(
                sockets.insert(endpoint.socket_path.clone()),
                "duplicate PCP runtime socket: {}",
                endpoint.socket_path.display()
            );
            anyhow::ensure!(
                principals.insert(endpoint.client_id.clone()),
                "duplicate PCP runtime Principal: {}",
                endpoint.client_id
            );
        }
        if let Some(maintenance) = &self.maintenance {
            maintenance.validate()?;
        }
        Ok(())
    }

    fn resolve_paths(&mut self, base: &Path) {
        if self.store_path.is_relative() {
            self.store_path = base.join(&self.store_path);
        }
        for endpoint in &mut self.endpoints {
            if endpoint.socket_path.is_relative() {
                endpoint.socket_path = base.join(&endpoint.socket_path);
            }
        }
        if let Some(maintenance) = &mut self.maintenance {
            maintenance.resolve_paths(base);
        }
    }
}

impl RuntimeEndpointConfig {
    pub fn access_session(
        &self,
        identity_id: &str,
        endpoint_index: usize,
    ) -> Result<AccessSession> {
        let mode = self.access_mode.parse::<AccessMode>()?;
        let scopes = self
            .allowed_scopes
            .iter()
            .map(|scope| scope.replace("{identity_id}", identity_id))
            .collect();
        Ok(mode.session(
            AccessPrincipal {
                principal_id: self.client_id.clone(),
                principal_type: parse_principal_type(&self.client_type)?,
                display_name: self.client_name.clone(),
            },
            self.session_id.clone().unwrap_or_else(|| {
                let started = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                format!(
                    "pcp-runtime:{}:{endpoint_index}:{started}",
                    std::process::id(),
                )
            }),
            scopes,
            self.allow_cross_scope_derivation,
        ))
    }
}

fn parse_principal_type(value: &str) -> Result<AccessPrincipalType> {
    match value {
        "host" => Ok(AccessPrincipalType::Host),
        "model_client" => Ok(AccessPrincipalType::ModelClient),
        "cli" => Ok(AccessPrincipalType::Cli),
        "service" => Ok(AccessPrincipalType::Service),
        other => anyhow::bail!("unsupported PCP client_type: {other}"),
    }
}

fn default_client_type() -> String {
    "service".to_owned()
}

fn default_access_mode() -> String {
    "read".to_owned()
}
