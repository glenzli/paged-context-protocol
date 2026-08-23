use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use pcp_runtime::{MaintenanceMode, RuntimeConfig};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};

const START_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(super) struct ManagedOptions {
    pub home: PathBuf,
    pub runtime_binary: PathBuf,
}

impl ManagedOptions {
    pub(super) fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Option<Self>> {
        let mut arguments = arguments.into_iter();
        let Some(first) = arguments.next() else {
            return Ok(None);
        };
        if first != "--managed" {
            anyhow::bail!("unknown pcp-console argument: {}", first.to_string_lossy());
        }

        let mut home = None;
        let mut runtime_binary = None;
        while let Some(argument) = arguments.next() {
            match argument.to_string_lossy().as_ref() {
                "--home" => {
                    home = Some(PathBuf::from(
                        arguments
                            .next()
                            .context("pcp-console --home requires a directory")?,
                    ));
                }
                "--runtime-binary" => {
                    runtime_binary = Some(PathBuf::from(
                        arguments
                            .next()
                            .context("pcp-console --runtime-binary requires a path")?,
                    ));
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => anyhow::bail!("unknown pcp-console managed argument: {other}"),
            }
        }

        Ok(Some(Self {
            home: home
                .or_else(|| env::var_os("PCP_HOME").map(PathBuf::from))
                .unwrap_or_else(default_home),
            runtime_binary: runtime_binary
                .or_else(|| env::var_os("PCP_RUNTIME_BINARY").map(PathBuf::from))
                .unwrap_or_else(default_runtime_binary),
        }))
    }
}

pub(super) struct ManagedRuntime {
    paths: ManagedPaths,
    runtime_binary: PathBuf,
    child: Mutex<Option<Child>>,
}

#[derive(Clone, Debug)]
pub(super) struct ManagedRuntimeStatus {
    pub managed: bool,
    pub owns_process: bool,
    pub pid: Option<u32>,
    pub home: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct MaintenanceSettings {
    pub enabled: bool,
    pub mode: MaintenanceMode,
    pub min_new_pages: usize,
    pub quiet_period_seconds: u64,
    pub max_wait_seconds: u64,
}

impl ManagedRuntime {
    pub(super) async fn start(options: ManagedOptions) -> Result<Arc<Self>> {
        let paths = ManagedPaths::prepare(options.home)?;
        let runtime = Arc::new(Self {
            paths,
            runtime_binary: options.runtime_binary,
            child: Mutex::new(None),
        });
        runtime.ensure_running().await?;
        Ok(runtime)
    }

    pub(super) fn operator_socket(&self) -> &Path {
        &self.paths.operator_socket
    }

    pub(super) fn runtime_config(&self) -> &Path {
        &self.paths.runtime_config
    }

    pub(super) async fn restart(&self) -> Result<ManagedRuntimeStatus> {
        let mut child = self.child.lock().await;
        if let Some(previous) = child.take() {
            stop_child(previous).await?;
        }
        self.start_locked(&mut child).await?;
        Ok(self.status_locked(&child))
    }

    pub(super) async fn update_maintenance_settings(
        &self,
        settings: MaintenanceSettings,
    ) -> Result<ManagedRuntimeStatus> {
        settings.validate()?;
        rewrite_maintenance_settings(&self.paths.runtime_config, &settings)?;
        self.restart().await
    }

    pub(super) async fn shutdown(&self) -> Result<()> {
        let mut child = self.child.lock().await;
        if let Some(child) = child.take() {
            stop_child(child).await?;
        }
        Ok(())
    }

    pub(super) async fn status(&self) -> ManagedRuntimeStatus {
        let mut child = self.child.lock().await;
        if child
            .as_mut()
            .is_some_and(|process| process.try_wait().ok().flatten().is_some())
        {
            *child = None;
        }
        self.status_locked(&child)
    }

    async fn ensure_running(&self) -> Result<()> {
        let mut child = self.child.lock().await;
        if child
            .as_mut()
            .is_some_and(|process| process.try_wait().ok().flatten().is_none())
        {
            return Ok(());
        }
        *child = None;
        self.start_locked(&mut child).await
    }

    async fn start_locked(&self, child: &mut Option<Child>) -> Result<()> {
        anyhow::ensure!(
            self.runtime_binary.is_file(),
            "PCP Runtime binary is not a regular file: {}",
            self.runtime_binary.display()
        );
        let mut command = Command::new(&self.runtime_binary);
        command
            .arg("--config")
            .arg(&self.paths.runtime_config)
            .kill_on_drop(true);
        let started = command
            .spawn()
            .with_context(|| format!("start PCP Runtime {}", self.runtime_binary.display()))?;
        *child = Some(started);
        let started_at = tokio::time::Instant::now();
        loop {
            if socket_ready(&self.paths.operator_socket).await {
                return Ok(());
            }
            if child
                .as_mut()
                .is_some_and(|process| process.try_wait().ok().flatten().is_some())
            {
                *child = None;
                anyhow::bail!("PCP Runtime exited before exposing its operator endpoint");
            }
            if started_at.elapsed() >= START_TIMEOUT {
                if let Some(child) = child.take() {
                    let _ = stop_child(child).await;
                }
                anyhow::bail!(
                    "PCP Runtime did not expose its operator endpoint within {} seconds",
                    START_TIMEOUT.as_secs()
                );
            }
            sleep(Duration::from_millis(150)).await;
        }
    }

    fn status_locked(&self, child: &Option<Child>) -> ManagedRuntimeStatus {
        ManagedRuntimeStatus {
            managed: true,
            owns_process: child.is_some(),
            pid: child.as_ref().and_then(Child::id),
            home: self.paths.home.clone(),
        }
    }
}

impl MaintenanceSettings {
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.min_new_pages > 0, "minimum new Pages must be positive");
        anyhow::ensure!(
            self.quiet_period_seconds > 0,
            "quiet period must be positive"
        );
        anyhow::ensure!(
            self.max_wait_seconds >= self.quiet_period_seconds,
            "maximum wait must not be shorter than the quiet period"
        );
        Ok(())
    }
}

fn rewrite_maintenance_settings(path: &Path, settings: &MaintenanceSettings) -> Result<()> {
    let original = fs::read_to_string(path)
        .with_context(|| format!("read managed Runtime configuration {}", path.display()))?;
    let mode = match settings.mode {
        MaintenanceMode::Observe => "\"observe\"",
        MaintenanceMode::Apply => "\"apply\"",
    };
    let rewritten = rewrite_toml_key(
        &original,
        "maintenance",
        "enabled",
        &settings.enabled.to_string(),
    )?;
    let rewritten = rewrite_toml_key(&rewritten, "maintenance", "mode", mode)?;
    let rewritten = rewrite_toml_key(
        &rewritten,
        "maintenance.write_trigger",
        "min_new_pages",
        &settings.min_new_pages.to_string(),
    )?;
    let rewritten = rewrite_toml_key(
        &rewritten,
        "maintenance.write_trigger",
        "quiet_period_seconds",
        &settings.quiet_period_seconds.to_string(),
    )?;
    let rewritten = rewrite_toml_key(
        &rewritten,
        "maintenance.write_trigger",
        "max_wait_seconds",
        &settings.max_wait_seconds.to_string(),
    )?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, rewritten).with_context(|| {
        format!(
            "write managed Runtime configuration {}",
            temporary.display()
        )
    })?;
    let config = RuntimeConfig::load(&temporary)?;
    config
        .maintenance
        .as_ref()
        .context("managed Runtime configuration has no maintenance section")?
        .validate()?;
    let permissions = fs::metadata(path)
        .with_context(|| format!("inspect managed Runtime configuration {}", path.display()))?
        .permissions();
    fs::set_permissions(&temporary, permissions).with_context(|| {
        format!(
            "secure managed Runtime configuration {}",
            temporary.display()
        )
    })?;
    fs::rename(&temporary, path)
        .with_context(|| format!("publish managed Runtime configuration {}", path.display()))
}

fn rewrite_toml_key(source: &str, section: &str, key: &str, value: &str) -> Result<String> {
    let header = format!("[{section}]");
    let mut output = String::with_capacity(source.len() + key.len() + value.len() + 8);
    let mut in_section = false;
    let mut section_found = false;
    let mut key_written = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == header {
            section_found = true;
            in_section = true;
            output.push_str(line);
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            if !key_written {
                output.push_str(&format!("{key} = {value}\n"));
                key_written = true;
            }
            in_section = false;
        }
        if in_section
            && trimmed
                .strip_prefix(key)
                .is_some_and(|suffix| suffix.trim_start().starts_with('='))
        {
            output.push_str(&format!("{key} = {value}\n"));
            key_written = true;
        } else {
            output.push_str(line);
        }
    }
    if section_found && in_section && !key_written {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&format!("{key} = {value}\n"));
    }
    if !section_found {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&format!("\n[{section}]\n{key} = {value}\n"));
    }
    anyhow::ensure!(
        section_found || output.contains(&header),
        "could not write managed Runtime configuration section [{section}]"
    );
    Ok(output)
}

#[derive(Clone, Debug)]
struct ManagedPaths {
    home: PathBuf,
    runtime_config: PathBuf,
    operator_socket: PathBuf,
}

impl ManagedPaths {
    fn prepare(home: PathBuf) -> Result<Self> {
        anyhow::ensure!(home.is_absolute(), "PCP home must be an absolute path");
        create_private_directory(&home)?;
        let config = home.join("config");
        let data = home.join("data");
        let run = home.join("run");
        for directory in [&config, &data, &run] {
            create_private_directory(directory)?;
        }
        let runtime_config = config.join("runtime.toml");
        let operator_socket = run.join("pcp-console.sock");
        if !runtime_config.exists() {
            let config = default_runtime_config(&data, &run);
            fs::write(&runtime_config, config).with_context(|| {
                format!("write PCP runtime config {}", runtime_config.display())
            })?;
            fs::set_permissions(&runtime_config, fs::Permissions::from_mode(0o600)).with_context(
                || format!("secure PCP runtime config {}", runtime_config.display()),
            )?;
        }
        Ok(Self {
            home,
            runtime_config,
            operator_socket,
        })
    }
}

fn default_runtime_config(data: &Path, run: &Path) -> String {
    format!(
        "# PCP owns this local Runtime configuration.\n# Add tenant-specific static endpoints only when a client cannot use enrollment.\n\nstore_path = \"{}\"\n\n[[endpoints]]\nsocket_path = \"{}\"\nclient_id = \"operator:local\"\nclient_type = \"service\"\nclient_name = \"PCP Console\"\naccess_mode = \"admin\"\nallowed_scopes = [\"user:{{identity_id}}\"]\nallow_cross_scope_derivation = true\n\n# Runtime owns maintenance cadence and state. Maintenance remains disabled until\n# PCP has an independently authorized semantic provider.\n#\n# [maintenance]\n# enabled = true\n# mode = \"observe\"\n# state_path = \"{}\"\n# allowed_scopes = [\"user:{{identity_id}}\"]\n# interval_seconds = 21600\n# max_interval_seconds = 86400\n# initial_delay_seconds = 300\n# max_jobs_per_cycle = 3\n# principal_id = \"service:pcp-maintainer\"\n# principal_name = \"PCP runtime maintainer\"\n#\n# [maintenance.worker]\n# provider = \"infer_runtime\"\n# credential_file = \"/absolute/path/to/pcp-runtime.token\"\n# timeout_seconds = 120\n# summary_deployment_id = \"codex_gpt_5_6_luna\"\n# reasoning_deployment_id = \"codex_gpt_5_6_luna\"\n# escalation_deployment_id = \"codex_gpt_5_6_sol\"\n# actor_id = \"model:infer-runtime-maintenance\"\n# actor_type = \"model\"\n#\n# [maintenance.relation]\n# enabled = true\n# candidate_window = 24\n# routing_chars_per_page = 800\n# retry_after_seconds = 86400\n#\n# Intent matching performs a bounded Router review over semantic candidates.\n# It is separate from Runtime maintenance and remains unavailable until an\n# Infer Runtime credential is configured.\n#\n# [intent_match]\n# credential_file = \"/absolute/path/to/pcp-runtime.token\"\n# timeout_seconds = 180\n# max_catalog_pages = 250\n",
        data.join("context.sqlite3").display(),
        run.join("pcp-console.sock").display(),
        data.join("maintenance-state.json").display(),
    )
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create PCP directory {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure PCP directory {}", path.display()))
}

async fn socket_ready(path: &Path) -> bool {
    timeout(
        Duration::from_millis(250),
        tokio::net::UnixStream::connect(path),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_some()
}

async fn stop_child(mut child: Child) -> Result<()> {
    if let Some(pid) = child.id() {
        // The Runtime handles SIGTERM by stopping endpoints, observer, and maintenance first.
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error).context("terminate PCP Runtime");
            }
        }
    }
    if timeout(STOP_TIMEOUT, child.wait()).await.is_err() {
        child.start_kill().context("force-stop PCP Runtime")?;
        child
            .wait()
            .await
            .context("wait for force-stopped PCP Runtime")?;
    }
    Ok(())
}

fn default_home() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/PCP"))
            .unwrap_or_else(|| PathBuf::from("/tmp/pcp"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("pcp")
    }
}

fn default_runtime_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("pcp-runtime")))
        .unwrap_or_else(|| PathBuf::from("pcp-runtime"))
}

fn print_help() {
    println!(
        "pcp-console [--managed [--home <directory>] [--runtime-binary <path>]]\n\nWithout --managed, PCP Console attaches to PCP_RUNTIME_SOCKET. With --managed, it owns a local pcp-runtime child and stores configuration, data, sockets, and maintenance state under PCP_HOME or the platform PCP home."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_home_initializes_a_private_runtime_layout() {
        let root = std::env::temp_dir().join(format!("pcp-console-managed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = ManagedPaths::prepare(root.clone()).expect("prepare managed paths");
        let config = fs::read_to_string(&paths.runtime_config).expect("read runtime config");
        assert!(config.contains(&paths.operator_socket.display().to_string()));
        assert!(config.contains("PCP owns this local Runtime configuration"));
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&paths.runtime_config)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn maintenance_settings_rewriter_preserves_other_runtime_configuration() {
        let source = "store_path = \"/tmp/context.sqlite3\"\n\n[maintenance]\nenabled = false\nmode = \"observe\"\nstate_path = \"/tmp/maintenance.json\"\n\n[maintenance.worker]\nprovider = \"infer_runtime\"\n";
        let settings = MaintenanceSettings {
            enabled: true,
            mode: MaintenanceMode::Apply,
            min_new_pages: 8,
            quiet_period_seconds: 600,
            max_wait_seconds: 3600,
        };
        let rewritten =
            rewrite_toml_key(source, "maintenance", "enabled", "true").expect("rewrite enabled");
        let rewritten =
            rewrite_toml_key(&rewritten, "maintenance", "mode", "\"apply\"").expect("rewrite mode");
        let rewritten = rewrite_toml_key(
            &rewritten,
            "maintenance.write_trigger",
            "min_new_pages",
            &settings.min_new_pages.to_string(),
        )
        .expect("add trigger minimum");
        assert!(rewritten.contains("store_path = \"/tmp/context.sqlite3\""));
        assert!(rewritten.contains("[maintenance.worker]\nprovider = \"infer_runtime\""));
        assert!(rewritten.contains("enabled = true"));
        assert!(rewritten.contains("mode = \"apply\""));
        assert!(rewritten.contains("[maintenance.write_trigger]\nmin_new_pages = 8"));
    }
}
