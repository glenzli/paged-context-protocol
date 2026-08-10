use std::{env, path::PathBuf, time::Duration};

use anyhow::{Context, Result};

const DEFAULT_REQUEST_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
pub struct EnrollmentConfig {
    pub enabled: bool,
    pub runtime_root: PathBuf,
    pub state_path: PathBuf,
    pub admin_socket_path: PathBuf,
    pub request_ttl: Duration,
}

impl EnrollmentConfig {
    pub fn from_env(
        runtime_root: PathBuf,
        store_path: PathBuf,
        runtime_socket_hint: PathBuf,
    ) -> Result<Self> {
        let enabled = env::var("PCP_ENROLLMENT_ENABLED")
            .map(|value| !matches!(value.as_str(), "0" | "false" | "no"))
            .unwrap_or(true);
        let state_path = env::var_os("PCP_ENROLLMENT_STATE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| store_path.with_file_name("pcp-enrollments.json"));
        let default_admin_socket = runtime_socket_hint
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("pcp-enrollment-admin.sock");
        let admin_socket_path = env::var_os("PCP_ENROLLMENT_ADMIN_SOCKET")
            .map(PathBuf::from)
            .unwrap_or(default_admin_socket);
        let request_ttl = env::var("PCP_ENROLLMENT_REQUEST_TTL_SECONDS")
            .ok()
            .map(|value| {
                value
                    .parse::<u64>()
                    .context("parse PCP_ENROLLMENT_REQUEST_TTL_SECONDS")
            })
            .transpose()?
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_REQUEST_TTL);
        anyhow::ensure!(
            !enabled || !runtime_root.as_os_str().is_empty(),
            "PCP enrollment requires an Infra Protocol runtime root"
        );
        anyhow::ensure!(
            !enabled || request_ttl >= Duration::from_secs(30),
            "PCP enrollment request TTL must be at least 30 seconds"
        );
        Ok(Self {
            enabled,
            runtime_root,
            state_path,
            admin_socket_path,
            request_ttl,
        })
    }

    #[cfg(test)]
    pub fn disabled_for_test(root: PathBuf) -> Self {
        Self {
            enabled: false,
            state_path: root.join("pcp-enrollments.json"),
            admin_socket_path: root.join("pcp-enrollment-admin.sock"),
            runtime_root: root,
            request_ttl: DEFAULT_REQUEST_TTL,
        }
    }

    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self {
        Self {
            enabled: true,
            state_path: root.join("pcp-enrollments.json"),
            admin_socket_path: root.join("pcp-enrollment-admin.sock"),
            runtime_root: root,
            request_ttl: Duration::from_secs(60),
        }
    }
}
