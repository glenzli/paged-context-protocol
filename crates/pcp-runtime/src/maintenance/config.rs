use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use pcp_client::AccessMode;
use pcp_core::{AccessPrincipal, AccessPrincipalType, AccessSession, Actor, ActorType};
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
    pub worker: WorkerCommandConfig,
    #[serde(default)]
    pub summary: SummaryMaintenanceConfig,
    #[serde(default)]
    pub compaction: CompactionMaintenanceConfig,
    #[serde(default)]
    pub retention: RetentionMaintenanceConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCommandConfig {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_worker_timeout_seconds")]
    pub timeout_seconds: u64,
    pub actor_id: String,
    #[serde(default = "default_worker_actor_type")]
    pub actor_type: String,
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
pub struct CompactionMaintenanceConfig {
    pub enabled: bool,
    pub candidate_window: usize,
    pub routing_chars_per_page: usize,
    pub max_pages_per_candidate: usize,
    pub max_input_chars: u32,
    pub retry_after_seconds: u64,
    pub excluded_page_kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionMaintenanceConfig {
    pub enabled: bool,
    pub apply: bool,
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
            apply: false,
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

impl Default for CompactionMaintenanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            candidate_window: 32,
            routing_chars_per_page: 480,
            max_pages_per_candidate: 8,
            max_input_chars: 64_000,
            retry_after_seconds: 86_400,
            excluded_page_kinds: vec![
                "pcp_summary".to_owned(),
                "validity_assessment".to_owned(),
                "conversation_event".to_owned(),
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
        anyhow::ensure!(
            !self.worker.program.as_os_str().is_empty(),
            "PCP maintenance worker program must not be empty"
        );
        anyhow::ensure!(
            self.worker.timeout_seconds > 0,
            "PCP maintenance worker timeout_seconds must be positive"
        );
        anyhow::ensure!(
            !self.worker.actor_id.trim().is_empty(),
            "PCP maintenance worker actor_id must not be empty"
        );
        anyhow::ensure!(
            matches!(
                ActorType::parse(&self.worker.actor_type),
                Some(ActorType::Model | ActorType::Tool)
            ),
            "unsupported PCP maintenance worker actor_type: {}",
            self.worker.actor_type
        );
        anyhow::ensure!(
            !self.summary.enabled || self.summary.minimum_chars > 0,
            "PCP summary maintenance minimum_chars must be positive"
        );
        anyhow::ensure!(
            !self.compaction.enabled || (2..=64).contains(&self.compaction.max_pages_per_candidate),
            "PCP compaction max_pages_per_candidate must be between 2 and 64"
        );
        anyhow::ensure!(
            !self.compaction.enabled || self.compaction.candidate_window >= 2,
            "PCP compaction candidate_window must be at least 2"
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
        if self.worker.program.is_relative() && self.worker.program.components().count() > 1 {
            self.worker.program = base.join(&self.worker.program);
        }
    }

    pub fn access_session(&self, owner_id: &str) -> AccessSession {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let scopes = self
            .allowed_scopes
            .iter()
            .map(|scope| scope.replace("{owner_id}", owner_id))
            .collect();
        let access_mode = if self.mode == MaintenanceMode::Apply
            || (self.retention.enabled && self.retention.apply)
        {
            AccessMode::Write
        } else {
            AccessMode::Read
        };
        access_mode.session(
            AccessPrincipal {
                principal_id: self.principal_id.clone(),
                principal_type: AccessPrincipalType::Service,
                display_name: Some(self.principal_name.clone()),
            },
            format!("pcp-maintenance:{}:{started}", std::process::id()),
            scopes,
            false,
        )
    }

    pub fn applies_changes(&self) -> bool {
        self.mode == MaintenanceMode::Apply
    }

    pub fn applies_retention_changes(&self) -> bool {
        self.retention.enabled && self.retention.apply
    }

    pub fn worker_actor(&self) -> Actor {
        Actor {
            actor_type: ActorType::parse(&self.worker.actor_type)
                .expect("validated maintenance worker actor type"),
            actor_id: self.worker.actor_id.clone(),
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
