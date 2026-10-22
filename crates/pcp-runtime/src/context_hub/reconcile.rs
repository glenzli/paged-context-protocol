//! Offline project routing changes preserve Activity identities and optimistic receipts.
use super::persistence::{HubState, LockedState};
use crate::ProjectScopeMerge;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashMap, fs, path::Path};

pub struct PreparedContextProjectReconciliation {
    db: LockedState,
    path: std::path::PathBuf,
    original: Vec<u8>,
    changes: Vec<Value>,
}

impl PreparedContextProjectReconciliation {
    pub async fn prepare(
        path: &Path,
        identity: &str,
        merges: &[ProjectScopeMerge],
    ) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let mut db = LockedState::open(path, identity).await?;
        let original = fs::read(path)?;
        let reread: HubState = serde_json::from_slice(&original)?;
        ensure!(
            serde_json::to_value(&db.state)? == serde_json::to_value(&reread)?,
            "context state changed while preparing project reconciliation"
        );
        let changes = rewrite(&mut db.state, merges)?;
        Ok(Some(Self {
            db,
            path: path.into(),
            original,
            changes,
        }))
    }

    pub fn summary(&self) -> Value {
        json!({"activityTransfers":self.changes})
    }

    pub fn verify_backup(&self, backup: &Path, merges: &[ProjectScopeMerge]) -> Result<()> {
        ensure!(
            backup.is_absolute() && fs::canonicalize(backup)? != fs::canonicalize(&self.path)?,
            "use a separate absolute context state backup"
        );
        let bytes = fs::read(backup)?;
        if bytes != self.original {
            let mut prior: HubState = serde_json::from_slice(&bytes)?;
            ensure!(
                prior.schema_version == 1 && prior.identity_id == self.db.state.identity_id,
                "context backup schema or Store identity mismatch"
            );
            rewrite(&mut prior, merges)?;
            ensure!(
                serde_json::to_value(prior)? == serde_json::to_value(&self.db.state)?,
                "context backup does not cover this migration"
            );
        }
        Ok(())
    }

    pub fn publish(self) -> Result<()> {
        ensure!(
            fs::read(&self.path)? == self.original,
            "context state changed during project reconciliation"
        );
        if !self.changes.is_empty() {
            self.db.save()?;
        }
        Ok(())
    }
}

fn rewrite(state: &mut HubState, merges: &[ProjectScopeMerge]) -> Result<Vec<Value>> {
    let redirects = merges
        .iter()
        .map(|m| (m.source_scope.as_str(), m.target_scope.as_str()))
        .collect::<HashMap<_, _>>();
    // Candidate promotion and synthesis retry inputs need their own reviewed migration.
    // Do not silently change their durable operation identity or historical receipts.
    let mut other = serde_json::to_value(&*state)?;
    other.as_object_mut().unwrap().remove("activity");
    ensure!(
        !contains_scope(&other, &redirects),
        "review non-Activity context references before merging project Scopes"
    );
    let mut changes = Vec::new();
    for card in &mut state.activity {
        if let Some(target) = redirects.get(card.scope.as_str()) {
            changes.push(
                json!({"cardId":card.card_id,"from":card.scope,"to":target,"version":card.version}),
            );
            card.scope = (*target).into();
        }
    }
    Ok(changes)
}

fn contains_scope(value: &Value, redirects: &HashMap<&str, &str>) -> bool {
    match value {
        Value::String(s) => redirects.contains_key(s.as_str()),
        Value::Array(items) => items.iter().any(|v| contains_scope(v, redirects)),
        Value::Object(items) => items.values().any(|v| contains_scope(v, redirects)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_hub::persistence::ActivityCard;

    fn fixture() -> (HubState, Vec<ProjectScopeMerge>) {
        let state = HubState {
            schema_version: 1,
            identity_id: "test".into(),
            sequence: 7,
            activity: vec![ActivityCard {
                card_id: "act_stable".into(),
                client_id: "model:test".into(),
                scope: "project:old".into(),
                topic_key: "project:test:work".into(),
                summary: "Still working".into(),
                title: None,
                status: None,
                next_step: None,
                parent_topic_key: None,
                version: 7,
                updated_at: "2026-10-08T00:00:00Z".into(),
                expires_at: "2026-10-10T00:00:00Z".into(),
            }],
            ..Default::default()
        };
        let merges = vec![ProjectScopeMerge {
            source_scope: "project:old".into(),
            target_scope: "project:canonical".into(),
        }];
        (state, merges)
    }

    #[tokio::test]
    async fn transfers_activity_without_changing_receipts_and_can_retry() {
        let (state, merges) = fixture();
        let root = std::env::temp_dir().join(format!("pcp-context-merge-{}", uuid::Uuid::new_v4()));
        let path = root.join("context.json");
        let mut db = LockedState::open(&path, "test").await.unwrap();
        db.state = state;
        db.save().unwrap();
        drop(db);
        let backup = root.join("before.json");
        fs::copy(&path, &backup).unwrap();
        let prepared = PreparedContextProjectReconciliation::prepare(&path, "test", &merges)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prepared.changes.len(), 1);
        prepared.verify_backup(&backup, &merges).unwrap();
        prepared.publish().unwrap();
        let repeated = PreparedContextProjectReconciliation::prepare(&path, "test", &merges)
            .await
            .unwrap()
            .unwrap();
        assert!(repeated.changes.is_empty());
        assert_eq!(repeated.db.state.activity[0].card_id, "act_stable");
        assert_eq!(repeated.db.state.activity[0].version, 7);
        assert_eq!(repeated.db.state.sequence, 7);
        repeated.verify_backup(&backup, &merges).unwrap();
        repeated.publish().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_to_rewrite_other_context_operations() {
        let (mut state, merges) = fixture();
        state.candidates.push(serde_json::from_value(json!({
            "candidateId":"cand_stable", "clientId":"model:test",
            "input":{"scope":"project:old","eventId":"event","title":"Pending evidence","content":"Keep exact retry input"},
            "createdAt":"2026-10-08T00:00:00Z","expiresAt":"2026-10-10T00:00:00Z",
            "version":1,"status":"pending","snoozedUntil":null,"reviewKey":null,
            "promotionRequest":null,"result":null
        })).unwrap());
        let before = serde_json::to_value(&state).unwrap();
        assert!(rewrite(&mut state, &merges).is_err());
        assert_eq!(serde_json::to_value(state).unwrap(), before);
    }
}
