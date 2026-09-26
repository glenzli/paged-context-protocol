//! Explicit, current-head-checked migration of selected sealed Pages or revisioned Topics.

use std::{collections::HashSet, env, fs, path::Path};

use anyhow::{Context, Result};
use pcp_core::{
    Actor, CreateScopeRequest, LifecycleStatus, PageMutability, Projection, ReadPage,
    ReadPagesRequest,
};
use pcp_sqlite::SqlitePcpStore;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    scopes: Vec<CreateScopeRequest>,
    transfers: Vec<Transfer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transfer {
    page_id: String,
    expected_revision_id: String,
    source_scope: String,
    target_scope: String,
}

pub async fn run(
    arguments: &mut impl Iterator<Item = String>,
    store_path: &Path,
    actor: Actor,
) -> Result<()> {
    let manifest_path = arguments
        .next()
        .context("scope-transfer requires a manifest path")?;
    let confirm = match arguments.next().as_deref() {
        None => false,
        Some("--confirm") => true,
        Some(other) => anyhow::bail!("unexpected scope-transfer argument: {other}"),
    };
    anyhow::ensure!(
        arguments.next().is_none(),
        "scope-transfer has too many arguments"
    );
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("read Scope transfer manifest {manifest_path}"))?,
    )?;
    anyhow::ensure!(
        manifest.schema_version == 1,
        "unsupported Scope transfer manifest version"
    );
    anyhow::ensure!(
        !manifest.transfers.is_empty() && manifest.transfers.len() <= 100,
        "Scope transfer manifest must contain 1-100 Pages"
    );
    let mut ids = HashSet::new();
    for transfer in &manifest.transfers {
        anyhow::ensure!(
            ids.insert(&transfer.page_id),
            "duplicate Page in Scope transfer manifest"
        );
        anyhow::ensure!(
            transfer.source_scope != transfer.target_scope,
            "Scope transfer source and destination are identical"
        );
    }
    anyhow::ensure!(store_path.is_file(), "Scope transfer Store does not exist");
    let store = SqlitePcpStore::open(store_path.to_path_buf()).await?;
    let existing_scopes = store.local_scope_names().await?;
    let declared_scopes = manifest
        .scopes
        .iter()
        .map(|scope| scope.namespace.as_str())
        .collect::<HashSet<_>>();
    for transfer in &manifest.transfers {
        anyhow::ensure!(
            existing_scopes.contains(&transfer.source_scope),
            "source Scope does not exist: {}",
            transfer.source_scope
        );
        anyhow::ensure!(
            existing_scopes.contains(&transfer.target_scope)
                || declared_scopes.contains(transfer.target_scope.as_str()),
            "destination Scope is neither existing nor declared: {}",
            transfer.target_scope
        );
    }

    // Preflight the whole manifest before creating a Scope or changing any Page.
    let mut originals = Vec::with_capacity(manifest.transfers.len());
    for transfer in &manifest.transfers {
        let page = read_one(&store, &transfer.page_id, &existing_scopes).await?;
        if page.page.namespace == transfer.target_scope
            && page.revision.previous_revision_id.as_deref()
                == Some(transfer.expected_revision_id.as_str())
        {
            originals.push(None);
            continue;
        }
        anyhow::ensure!(
            page.page.namespace == transfer.source_scope,
            "source Scope changed for {}",
            transfer.page_id
        );
        anyhow::ensure!(
            page.revision.revision_id == transfer.expected_revision_id,
            "source head changed for {}",
            transfer.page_id
        );
        anyhow::ensure!(
            (page.page.mutability == PageMutability::Sealed
                || (page.page.kind == "topic_summary"
                    && page.page.mutability == PageMutability::Revisioned))
                && page.page.lifecycle_status == LifecycleStatus::Active,
            "only active sealed Pages or revisioned Topics can be transferred: {}",
            transfer.page_id
        );
        anyhow::ensure!(
            page.validity.is_none(),
            "Page has a validity decision requiring separate review: {}",
            transfer.page_id
        );
        originals.push(Some(page));
    }
    let preview = manifest
        .transfers
        .iter()
        .zip(&originals)
        .map(|(transfer, original)| {
            json!({
                "pageId": transfer.page_id,
                "from": transfer.source_scope,
                "to": transfer.target_scope,
                "status": if original.is_some() { "ready" } else { "already_transferred" },
            })
        })
        .collect::<Vec<_>>();
    if !confirm {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store": store_path,
                "createScopes": manifest.scopes,
                "transfers": preview,
                "applied": false,
            }))?
        );
        return Ok(());
    }

    anyhow::ensure!(
        env::var_os("PCP_SCOPE_TRANSFER_BACKUP").is_some_and(|path| Path::new(&path).is_file()),
        "scope-transfer --confirm requires PCP_SCOPE_TRANSFER_BACKUP pointing to an existing backup file"
    );
    for scope in manifest.scopes {
        if !existing_scopes.contains(&scope.namespace) {
            store.create_scope(scope).await?;
        }
    }
    let allowed_scopes = store.local_scope_names().await?;
    let mut receipts = Vec::new();
    for (transfer, original) in manifest.transfers.into_iter().zip(originals) {
        let Some(original) = original else {
            receipts.push(json!({"pageId": transfer.page_id, "status": "already_transferred"}));
            continue;
        };
        let key = format!(
            "scope-transfer-v1:{}:{}:{}",
            transfer.page_id, transfer.expected_revision_id, transfer.target_scope
        );
        let written = store
            .transfer_page_scope(
                transfer.page_id.clone(),
                transfer.expected_revision_id.clone(),
                transfer.source_scope,
                transfer.target_scope.clone(),
                actor.clone(),
                key,
                allowed_scopes.clone(),
            )
            .await?;
        let current = read_one(&store, &transfer.page_id, &allowed_scopes).await?;
        anyhow::ensure!(
            current.page.namespace == transfer.target_scope
                && current.revision.previous_revision_id.as_deref()
                    == Some(transfer.expected_revision_id.as_str())
                && serde_json::to_value(&current.revision.payload)?
                    == serde_json::to_value(&original.revision.payload)?
                && serde_json::to_value(&current.revision.source_refs)?
                    == serde_json::to_value(&original.revision.source_refs)?
                && current.revision.facets == original.revision.facets
                && current.revision.observed_at == original.revision.observed_at,
            "Scope transfer readback differs from original Page: {}",
            transfer.page_id
        );
        receipts.push(
            json!({"pageId": written.page_id, "revisionId": written.revision_id,
            "scope": current.page.namespace, "status": "transferred"}),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"applied": true, "receipts": receipts}))?
    );
    Ok(())
}

async fn read_one(store: &SqlitePcpStore, page_id: &str, scopes: &[String]) -> Result<ReadPage> {
    store
        .read_pages(
            ReadPagesRequest {
                page_ids: vec![page_id.to_owned()],
                revision_ids: Vec::new(),
                projections: vec![
                    Projection::Manifest,
                    Projection::Payload,
                    Projection::Sources,
                    Projection::Facets,
                    Projection::Validity,
                ],
                max_chars: 64_000,
            },
            scopes.to_vec(),
        )
        .await?
        .into_iter()
        .next()
        .with_context(|| format!("Page is not readable: {page_id}"))
}
