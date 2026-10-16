//! Explicit offline operator reconciliation. Enrollment owns bindings and grant projection.
use super::{
    service::validate_loaded_state,
    state::{EnrollmentState, StateFile, StoredDecision},
};
use anyhow::{Context, Result};
use pcp_client::project_scope::canonical_project_key;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectReconciliationGroup {
    pub canonical_key: String,
    pub member_keys: Vec<String>,
    pub target_scope: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScopeMerge {
    pub source_scope: String,
    pub target_scope: String,
}

pub struct PreparedProjectReconciliation {
    path: PathBuf,
    original: Vec<u8>,
    file: StateFile,
    next: EnrollmentState,
    merges: Vec<ProjectScopeMerge>,
}

impl PreparedProjectReconciliation {
    pub fn prepare(path: &Path, groups: &[ProjectReconciliationGroup]) -> Result<Self> {
        anyhow::ensure!(
            path.is_absolute() && path.is_file(),
            "reconciliation requires an existing absolute enrollment path"
        );
        anyhow::ensure!(
            !groups.is_empty() && groups.len() <= 100,
            "reconciliation requires 1-100 explicit groups"
        );
        let (file, mut next) = StateFile::load(path.to_path_buf())?;
        let original = fs::read(path)?;
        let reread: EnrollmentState = serde_json::from_slice(&original)?;
        anyhow::ensure!(
            serde_json::to_value(&next)? == serde_json::to_value(reread)?,
            "enrollment changed while preparing reconciliation"
        );
        let projects = next.projects.clone();
        let mut used = HashSet::new();
        let mut replacements = HashMap::new();
        let mut redirects = HashMap::new();
        for group in groups {
            let canonical = canonical_project_key(&group.canonical_key)?;
            anyhow::ensure!(
                canonical == group.canonical_key && group.member_keys.len() >= 2,
                "group requires a canonical key and at least two reviewed member keys"
            );
            let mut indices = BTreeSet::new();
            for key in &group.member_keys {
                let matches = projects
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.project_key == *key || p.aliases.contains(key))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    matches.len() == 1,
                    "member key is missing or ambiguous: {key}"
                );
                indices.insert(matches[0]);
            }
            let target_index = *indices
                .iter()
                .find(|&&i| projects[i].scope == group.target_scope)
                .context("target Scope must belong to a reviewed member")?;
            let mut binding = projects[target_index].clone();
            let mut aliases = BTreeSet::new();
            let mut history = BTreeSet::new();
            for index in &indices {
                anyhow::ensure!(used.insert(*index), "overlapping reconciliation groups");
                let p = &projects[*index];
                anyhow::ensure!(
                    p.ready && p.registration_id == binding.registration_id,
                    "review provisioning or cross-client ownership before merging"
                );
                aliases.insert(p.project_key.clone());
                aliases.extend(p.aliases.iter().cloned());
                history.insert(p.scope.clone());
                history.extend(p.historical_scopes.iter().cloned());
            }
            aliases.remove(&canonical);
            history.remove(&group.target_scope);
            binding.project_key = canonical;
            binding.aliases = aliases.into_iter().collect();
            binding.historical_scopes = history.into_iter().collect();
            for scope in &binding.historical_scopes {
                anyhow::ensure!(
                    redirects
                        .insert(scope.clone(), binding.scope.clone())
                        .is_none(),
                    "historical Scope belongs to multiple groups"
                );
            }
            replacements.insert(target_index, binding);
        }
        next.projects = projects
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut p)| {
                if let Some(binding) = replacements.remove(&i) {
                    return Some(binding);
                }
                if used.contains(&i) {
                    return None;
                }
                match canonical_project_key(&p.project_key) {
                    Ok(key) if key != p.project_key => {
                        p.aliases.push(p.project_key.clone());
                        p.project_key = key;
                    }
                    _ => {}
                }
                Some(p)
            })
            .collect();
        for registration in &mut next.registrations {
            rewrite_scopes(&mut registration.project_scopes, &redirects);
            rewrite_scopes(&mut registration.approved_access.scopes, &redirects);
            for p in &next.projects {
                if registration.project_scopes.contains(&p.scope) {
                    registration
                        .historical_project_scopes
                        .extend(p.historical_scopes.iter().cloned());
                }
            }
            registration.historical_project_scopes.sort();
            registration.historical_project_scopes.dedup();
        }
        for request in &mut next.requests {
            if matches!(request.decision, StoredDecision::Approved { .. }) {
                rewrite_scopes(&mut request.requested_access.scopes, &redirects);
            }
        }
        validate_loaded_state(&next)?;
        let mut merges = redirects
            .into_iter()
            .map(|(source_scope, target_scope)| ProjectScopeMerge {
                source_scope,
                target_scope,
            })
            .collect::<Vec<_>>();
        merges.sort_by(|a, b| a.source_scope.cmp(&b.source_scope));
        Ok(Self {
            path: path.into(),
            original,
            file,
            next,
            merges,
        })
    }

    pub fn merges(&self) -> &[ProjectScopeMerge] {
        &self.merges
    }

    pub fn verify_backup(
        &self,
        backup: &Path,
        groups: &[ProjectReconciliationGroup],
    ) -> Result<()> {
        anyhow::ensure!(
            backup.is_absolute() && backup != self.path,
            "use a separate absolute enrollment backup"
        );
        if fs::read(backup)? != self.original {
            // An identical retry after publication compares the same projected state, without overwriting later changes.
            let prior = Self::prepare(backup, groups)?;
            anyhow::ensure!(
                serde_json::to_value(&prior.next)? == serde_json::to_value(&self.next)?,
                "enrollment backup does not cover this migration"
            );
        }
        Ok(())
    }

    /// No credentials or request bodies enter the reviewable migration report.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"projects":self.next.projects.iter().map(|p|serde_json::json!({"projectKey":p.project_key,"scope":p.scope,"aliases":p.aliases,"historicalScopes":p.historical_scopes})).collect::<Vec<_>>(),"scopeMerges":self.merges})
    }

    pub fn publish(self) -> Result<()> {
        anyhow::ensure!(
            fs::read(&self.path)? == self.original,
            "enrollment changed during reconciliation; stop and prepare again"
        );
        self.file.write(&self.next)
    }
}

fn rewrite_scopes(scopes: &mut Vec<String>, redirects: &HashMap<String, String>) {
    for scope in scopes.iter_mut() {
        if let Some(target) = redirects.get(scope) {
            *scope = target.clone();
        }
    }
    // The approved request and registration must retain the same grant ordering.
    let mut seen = HashSet::new();
    scopes.retain(|scope| seen.insert(scope.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> (PathBuf, Vec<ProjectReconciliationGroup>) {
        let root =
            std::env::temp_dir().join(format!("pcp-reconcile-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("enrollments.json");
        let at = "2026-10-08T00:00:00Z";
        let project = |key: &str, scope: &str| serde_json::json!({"project_key":key,"scope":scope,"display_name":"PCP","description":null,"ready":true,"created_at":at,"registration_id":"reg_test"});
        let access = serde_json::json!({"mode":"contribute","scopes":["user:self","project:canonical","project:duplicate"],"read_all_scopes":false,"allow_cross_scope_derivation":false});
        let client = serde_json::json!({"principal":{"principalId":"model:test","principalType":"model_client","displayName":"Test"}});
        let state = serde_json::json!({"schema":"pcp.runtime.enrollment.state","schema_version":"20260810.1","requests":[{
            "request_id":"req_test","client":client,"requested_access":access,"credential_hash":"ab".repeat(32),"requested_at":at,"expires_at":"2026-10-09T00:00:00Z","decision":{"status":"approved","registration_id":"reg_test"}
        }],"registrations":[{
            "registration_id":"reg_test","client":{"principal":{"principalId":"model:test","principalType":"model_client","displayName":"Test"}},
            "approved_access":access,
            "allow_project_registration":true,"project_scopes":["project:canonical","project:duplicate"],"credential_hash":"ab".repeat(32),"created_at":at,"last_opened_at":null,"revoked_at":null
        }],"projects":[project("github.com/test/pcp","project:canonical"),project("pcp","project:duplicate")]});
        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        (
            path,
            vec![ProjectReconciliationGroup {
                canonical_key: "github.com/test/pcp".into(),
                member_keys: vec!["github.com/test/pcp".into(), "pcp".into()],
                target_scope: "project:canonical".into(),
            }],
        )
    }

    #[test]
    fn reconciliation_preserves_aliases_read_grants_and_identical_retry() {
        let (path, groups) = fixture();
        let backup = path.with_file_name("before.json");
        fs::copy(&path, &backup).unwrap();
        let prepared = PreparedProjectReconciliation::prepare(&path, &groups).unwrap();
        assert_eq!(prepared.next.projects.len(), 1);
        assert_eq!(
            prepared.next.registrations[0].approved_access.scopes,
            vec!["user:self", "project:canonical"]
        );
        assert_eq!(
            prepared.next.registrations[0].approved_access,
            prepared.next.requests[0].requested_access
        );
        assert_eq!(prepared.next.projects[0].aliases, vec!["pcp"]);
        assert_eq!(
            prepared.next.registrations[0].project_scopes,
            vec!["project:canonical"]
        );
        assert_eq!(
            prepared.next.registrations[0].historical_project_scopes,
            vec!["project:duplicate"]
        );
        prepared.verify_backup(&backup, &groups).unwrap();
        prepared.publish().unwrap();
        let repeated = PreparedProjectReconciliation::prepare(&path, &groups).unwrap();
        repeated.verify_backup(&backup, &groups).unwrap();
        assert_eq!(repeated.merges.len(), 1);
        repeated.publish().unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reconciliation_does_not_overwrite_changed_state_or_accept_overlapping_groups() {
        let (path, groups) = fixture();
        let prepared = PreparedProjectReconciliation::prepare(&path, &groups).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
        assert!(prepared.publish().is_err());
        let mut overlapping = groups.clone();
        overlapping.extend(groups);
        assert!(PreparedProjectReconciliation::prepare(&path, &overlapping).is_err());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
