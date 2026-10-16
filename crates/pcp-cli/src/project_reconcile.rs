//! Offline operator orchestration; identity/grant policy stays in enrollment.
use anyhow::{Context, Result};
use pcp_core::Actor;
use pcp_runtime::context_hub::{ContextHub, PreparedContextProjectReconciliation};
use pcp_runtime::{PreparedProjectReconciliation, ProjectReconciliationGroup};
use pcp_sqlite::SqlitePcpStore;
use serde::Deserialize;
use std::{
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    enrollment_path: PathBuf,
    enrollment_backup: PathBuf,
    #[serde(default)]
    context_backup: Option<PathBuf>,
    runtime_socket: PathBuf,
    scope_transfer_manifest: PathBuf,
    groups: Vec<ProjectReconciliationGroup>,
}

pub async fn run(
    arguments: &mut impl Iterator<Item = String>,
    store_path: &Path,
    actor: Actor,
) -> Result<()> {
    let path = arguments
        .next()
        .context("project-reconcile requires a manifest")?;
    let confirm = match arguments.next().as_deref() {
        None => false,
        Some("--confirm") => true,
        _ => anyhow::bail!("unexpected project-reconcile argument"),
    };
    anyhow::ensure!(
        arguments.next().is_none(),
        "too many project-reconcile arguments"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(
        manifest.schema_version == 1 && manifest.runtime_socket.is_absolute(),
        "invalid project-reconcile manifest"
    );
    if confirm {
        match UnixStream::connect(&manifest.runtime_socket) {
            Ok(_) => {
                anyhow::bail!("stop the Console-managed Runtime before project reconciliation")
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) => {}
            Err(e) => return Err(e).context("verify Runtime is stopped"),
        }
    }
    let prepared =
        PreparedProjectReconciliation::prepare(&manifest.enrollment_path, &manifest.groups)?;
    let transfers: super::scope_transfer::Manifest =
        serde_json::from_slice(&fs::read(&manifest.scope_transfer_manifest)?)?;
    anyhow::ensure!(
        transfers.schema_version == 1 && transfers.scopes.is_empty(),
        "project reconciliation only reuses reviewed existing Scopes"
    );
    for transfer in &transfers.transfers {
        anyhow::ensure!(
            prepared
                .merges()
                .iter()
                .any(|m| m.source_scope == transfer.source_scope
                    && m.target_scope == transfer.target_scope),
            "transfer is outside reviewed project merges"
        );
    }
    let mut report = prepared.summary();
    let store = SqlitePcpStore::open(store_path.into()).await?;
    let context = PreparedContextProjectReconciliation::prepare(
        &ContextHub::state_path(store_path),
        store.identity_id(),
        prepared.merges(),
    )
    .await?;
    report["context"] = context
        .as_ref()
        .map(|c| c.summary())
        .unwrap_or_else(|| serde_json::json!({"activityTransfers":[]}));
    if confirm {
        prepared.verify_backup(&manifest.enrollment_backup, &manifest.groups)?;
        if let Some(context) = &context {
            context.verify_backup(
                manifest
                    .context_backup
                    .as_deref()
                    .context("project reconciliation requires a context state backup")?,
                prepared.merges(),
            )?;
        }
    }
    let transfer_report = if transfers.transfers.is_empty() {
        serde_json::json!({"applied":confirm,"receipts":[]})
    } else {
        super::scope_transfer::execute(
            &manifest.scope_transfer_manifest,
            store_path,
            actor,
            confirm,
        )
        .await?
    };
    report["transfers"] = transfer_report;
    if confirm {
        anyhow::ensure!(
            std::env::var_os("PCP_SCOPE_TRANSFER_BACKUP").is_some_and(|p| Path::new(&p).is_file()),
            "project reconciliation requires a Store backup"
        );
        store
            .verify_scope_transfer_backup(Path::new(
                &std::env::var_os("PCP_SCOPE_TRANSFER_BACKUP").unwrap(),
            ))
            .await?;
        let mut retired = Vec::new();
        for merge in prepared.merges() {
            let changed = store
                .retire_project_scope(merge.source_scope.clone(), merge.target_scope.clone())
                .await?;
            retired.push(serde_json::json!({"scope":merge.source_scope,"canonicalScope":merge.target_scope,"changed":changed}));
        }
        if let Some(context) = context {
            context.publish()?;
        }
        prepared.publish()?;
        report["retiredScopes"] = retired.into();
    }
    report["applied"] = confirm.into();
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
