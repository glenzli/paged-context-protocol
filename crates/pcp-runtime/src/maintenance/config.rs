use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use pcp_client::AccessMode;
use pcp_core::{
    AccessPermission, AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceMode {
    #[default]
    Observe,
    Apply,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: MaintenanceMode,
    pub state_path: PathBuf,
    pub allowed_scopes: Vec<String>,
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: u64,
    #[serde(default)]
    pub initial_delay_seconds: u64,
    #[serde(default = "default_jobs_per_cycle")]
    pub max_jobs_per_cycle: u32,
    #[serde(default = "default_principal_id")]
    pub principal_id: String,
    #[serde(default = "default_principal_name")]
    pub principal_name: String,
    pub worker: MaintenanceWorkerConfig,
    #[serde(default)]
    pub summary: SummaryMaintenanceConfig,
    #[serde(default)]
    pub packing: PackingMaintenanceConfig,
    #[serde(default)]
    pub relation: RelationMaintenanceConfig,
    #[serde(default)]
    pub retention: RetentionMaintenanceConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceWorkerConfig {
    Command {
        program: PathBuf,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "default_worker_timeout_seconds")]
        timeout_seconds: u64,
        actor_id: String,
        #[serde(default = "default_worker_actor_type")]
        actor_type: String,
    },
    InferRuntime {
        credential_file: PathBuf,
        #[serde(default = "default_worker_timeout_seconds")]
        timeout_seconds: u64,
        #[serde(default = "default_infer_summary_deployment_id")]
        summary_deployment_id: String,
        #[serde(default = "default_infer_reasoning_deployment_id")]
        reasoning_deployment_id: String,
        #[serde(default)]
        relation_deployment_id: Option<String>,
        actor_id: String,
        #[serde(default = "default_worker_actor_type")]
        actor_type: String,
    },
}

impl MaintenanceWorkerConfig {
    pub fn timeout_seconds(&self) -> u64 {
        match self {
            Self::Command {
                timeout_seconds, ..
            }
            | Self::InferRuntime {
                timeout_seconds, ..
            } => *timeout_seconds,
        }
    }

    pub fn actor_id(&self) -> &str {
        match self {
            Self::Command { actor_id, .. } | Self::InferRuntime { actor_id, .. } => actor_id,
        }
    }

    pub fn actor_type(&self) -> &str {
        match self {
            Self::Command { actor_type, .. } | Self::InferRuntime { actor_type, .. } => actor_type,
        }
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.timeout_seconds() > 0,
            "PCP maintenance worker timeout_seconds must be positive"
        );
        anyhow::ensure!(
            !self.actor_id().trim().is_empty(),
            "PCP maintenance worker actor_id must not be empty"
        );
        anyhow::ensure!(
            matches!(
                ActorType::parse(self.actor_type()),
                Some(ActorType::Model | ActorType::Tool)
            ),
            "unsupported PCP maintenance worker actor_type: {}",
            self.actor_type()
        );
        match self {
            Self::Command { program, .. } => anyhow::ensure!(
                !program.as_os_str().is_empty(),
                "PCP command maintenance worker program must not be empty"
            ),
            Self::InferRuntime {
                credential_file,
                summary_deployment_id,
                reasoning_deployment_id,
                relation_deployment_id,
                ..
            } => {
                anyhow::ensure!(
                    !credential_file.as_os_str().is_empty(),
                    "PCP Infer Runtime credential_file must not be empty"
                );
                anyhow::ensure!(
                    !summary_deployment_id.trim().is_empty(),
                    "PCP Infer Runtime summary_deployment_id must not be empty"
                );
                anyhow::ensure!(
                    !reasoning_deployment_id.trim().is_empty(),
                    "PCP Infer Runtime reasoning_deployment_id must not be empty"
                );
                anyhow::ensure!(
                    relation_deployment_id
                        .as_ref()
                        .is_none_or(|deployment_id| !deployment_id.trim().is_empty()),
                    "PCP Infer Runtime relation_deployment_id must not be empty"
                );
            }
        }
        Ok(())
    }

    fn resolve_paths(&mut self, base: &Path) {
        match self {
            Self::Command { program, .. }
                if program.is_relative() && program.components().count() > 1 =>
            {
                *program = base.join(&*program);
            }
            Self::InferRuntime {
                credential_file, ..
            } if credential_file.is_relative() => {
                *credential_file = base.join(&*credential_file);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SummaryMaintenanceConfig {
    pub enabled: bool,
    pub minimum_chars: usize,
    pub max_input_chars: u32,
    pub retry_after_seconds: u64,
    pub excluded_page_kinds: Vec<String>,
}

impl Default for SummaryMaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            minimum_chars: 4_000,
            max_input_chars: 32_000,
            retry_after_seconds: 86_400,
            excluded_page_kinds: vec![
                "pcp_summary".to_owned(),
                "summary_projection".to_owned(),
                "validity_assessment".to_owned(),
                "conversation_event".to_owned(),
                "tombstone".to_owned(),
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PackingMaintenanceConfig {
    pub enabled: bool,
    pub max_pages: usize,
    pub max_input_chars: u32,
    pub excluded_page_kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RelationMaintenanceConfig {
    pub enabled: bool,
    pub candidate_window: usize,
    pub routing_chars_per_page: usize,
    pub retry_after_seconds: u64,
    pub excluded_page_kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionMaintenanceConfig {
    pub enabled: bool,
    pub write_leases: bool,
    pub minimum_age_days: u32,
    pub keep_recent_revisions_per_page: u32,
    pub candidate_window: usize,
    pub routing_chars_per_page: usize,
    pub max_revisions_per_cycle: usize,
    pub lease_days: u32,
    pub retry_after_seconds: u64,
    pub excluded_page_kinds: Vec<String>,
}

impl Default for RetentionMaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            write_leases: false,
            minimum_age_days: 30,
            keep_recent_revisions_per_page: 2,
            candidate_window: 32,
            routing_chars_per_page: 480,
            max_revisions_per_cycle: 4,
            lease_days: 90,
            retry_after_seconds: 30 * 86_400,
            excluded_page_kinds: vec![
                "pcp_summary".to_owned(),
                "summary_projection".to_owned(),
                "validity_assessment".to_owned(),
                "conversation_event".to_owned(),
                "tombstone".to_owned(),
            ],
        }
    }
}

impl Default for PackingMaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_pages: 8,
            max_input_chars: 64_000,
            excluded_page_kinds: vec![
                "pcp_summary".to_owned(),
                "summary_projection".to_owned(),
                "validity_assessment".to_owned(),
                "tombstone".to_owned(),
            ],
        }
    }
}

impl Default for RelationMaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            candidate_window: 24,
            routing_chars_per_page: 800,
            retry_after_seconds: 86_400,
            excluded_page_kinds: vec![
                "pcp_summary".to_owned(),
                "summary_projection".to_owned(),
                "validity_assessment".to_owned(),
                "tombstone".to_owned(),
            ],
        }
    }
}

impl MaintenanceConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        anyhow::ensure!(
            !self.allowed_scopes.is_empty(),
            "enabled PCP maintenance requires at least one allowed Scope"
        );
        anyhow::ensure!(
            self.allowed_scopes
                .iter()
                .all(|scope| !scope.trim().is_empty()),
            "PCP maintenance contains an empty Scope"
        );
        anyhow::ensure!(
            self.interval_seconds > 0,
            "PCP maintenance interval_seconds must be positive"
        );
        anyhow::ensure!(
            self.max_jobs_per_cycle > 0,
            "PCP maintenance max_jobs_per_cycle must be positive"
        );
        anyhow::ensure!(
            !self.principal_id.trim().is_empty(),
            "PCP maintenance principal_id must not be empty"
        );
        self.worker.validate()?;
        anyhow::ensure!(
            !self.summary.enabled || self.summary.minimum_chars > 0,
            "PCP summary maintenance minimum_chars must be positive"
        );
        anyhow::ensure!(
            !self.packing.enabled || (2..=64).contains(&self.packing.max_pages),
            "PCP packing max_pages must be between 2 and 64"
        );
        anyhow::ensure!(
            !self.packing.enabled || self.packing.max_input_chars > 0,
            "PCP packing max_input_chars must be positive"
        );
        anyhow::ensure!(
            !self.relation.enabled || (2..=64).contains(&self.relation.candidate_window),
            "PCP relation candidate_window must be between 2 and 64"
        );
        anyhow::ensure!(
            !self.relation.enabled || (1..=4_096).contains(&self.relation.routing_chars_per_page),
            "PCP relation routing_chars_per_page must be between 1 and 4096"
        );
        anyhow::ensure!(
            !self.relation.enabled || self.relation.retry_after_seconds > 0,
            "PCP relation retry_after_seconds must be positive"
        );
        anyhow::ensure!(
            !self.retention.enabled || self.retention.candidate_window > 0,
            "PCP retention candidate_window must be positive"
        );
        anyhow::ensure!(
            !self.retention.enabled || self.retention.routing_chars_per_page > 0,
            "PCP retention routing_chars_per_page must be positive"
        );
        anyhow::ensure!(
            !self.retention.enabled || (1..=64).contains(&self.retention.max_revisions_per_cycle),
            "PCP retention max_revisions_per_cycle must be between 1 and 64"
        );
        anyhow::ensure!(
            !self.retention.enabled || (1..=3_650).contains(&self.retention.lease_days),
            "PCP retention lease_days must be between 1 and 3650"
        );
        Ok(())
    }

    pub(crate) fn resolve_paths(&mut self, base: &Path) {
        if self.state_path.is_relative() {
            self.state_path = base.join(&self.state_path);
        }
        self.worker.resolve_paths(base);
    }

    pub fn access_session(&self, identity_id: &str) -> AccessSession {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let scopes = self
            .allowed_scopes
            .iter()
            .map(|scope| scope.replace("{identity_id}", identity_id))
            .collect();
        let access_mode = if self.mode == MaintenanceMode::Apply {
            AccessMode::Write
        } else {
            AccessMode::Read
        };
        let mut session = access_mode.session(
            AccessPrincipal {
                principal_id: self.principal_id.clone(),
                principal_type: AccessPrincipalType::Service,
                display_name: Some(self.principal_name.clone()),
            },
            format!("pcp-maintenance:{}:{started}", std::process::id()),
            scopes,
            false,
        );
        if self.retention.enabled {
            for grant in &mut session.grants {
                grant.permissions.push(AccessPermission::Audit);
            }
        }
        if self.packing.enabled && self.applies_changes() {
            for grant in &mut session.grants {
                grant.permissions.push(AccessPermission::Collect);
            }
        }
        session
    }

    pub fn applies_changes(&self) -> bool {
        self.mode == MaintenanceMode::Apply
    }

    pub fn writes_retention_leases(&self) -> bool {
        self.applies_changes() && self.retention.enabled && self.retention.write_leases
    }

    pub fn worker_actor(&self) -> Actor {
        Actor {
            actor_type: ActorType::parse(self.worker.actor_type())
                .expect("validated maintenance worker actor type"),
            actor_id: self.worker.actor_id().to_owned(),
        }
    }
}

fn default_interval_seconds() -> u64 {
    1_800
}

fn default_jobs_per_cycle() -> u32 {
    2
}

fn default_principal_id() -> String {
    "service:pcp-maintainer".to_owned()
}

fn default_principal_name() -> String {
    "PCP runtime maintainer".to_owned()
}

fn default_worker_timeout_seconds() -> u64 {
    120
}

fn default_worker_actor_type() -> String {
    "model".to_owned()
}

fn default_infer_summary_deployment_id() -> String {
    "ollama_qwen3_5_4b".to_owned()
}

fn default_infer_reasoning_deployment_id() -> String {
    "codex_gpt_5_6_luna".to_owned()
}
