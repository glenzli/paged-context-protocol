//! Registration policy, durable project bindings and recoverable Store provisioning.
use super::super::state::StoredProject;
use super::*;
use pcp_client::project_scope::{ProjectScopeRequest, ProjectScopeResult, canonical_project_key};
use pcp_core::AccessPermission;
use pcp_rpc::{EnsureProjectScopeParams, ProjectRegistrationPolicyParams};

const MAX_PROJECTS: usize = 1024;
const MAX_CLIENT_PROJECTS: usize = 256;

fn validate_request(project: &ProjectScopeRequest) -> Result<()> {
    canonical_project_key(&project.project_key)?;
    for value in [&project.display_name, &project.description]
        .into_iter()
        .flatten()
    {
        anyhow::ensure!(
            !value.trim().is_empty() && value.chars().count() <= 500,
            "project metadata must contain 1-500 characters"
        );
    }
    if let Some(scope) = &project.existing_scope {
        anyhow::ensure!(
            !scope.starts_with("user:") && !scope.trim().is_empty() && scope.len() <= 128,
            "a user Scope cannot be adopted as a project"
        );
    }
    Ok(())
}

pub(super) fn validate_projects(state: &EnrollmentState) -> Result<()> {
    anyhow::ensure!(
        state.projects.len() <= MAX_PROJECTS,
        "too many registered projects"
    );
    let mut keys = HashMap::new();
    let mut scopes = HashSet::new();
    for project in &state.projects {
        anyhow::ensure!(
            state
                .registrations
                .iter()
                .any(|r| r.registration_id == project.registration_id),
            "project owner registration is missing"
        );
        validate_request(&ProjectScopeRequest {
            project_key: project.project_key.clone(),
            display_name: Some(project.display_name.clone()),
            description: project.description.clone(),
            existing_scope: Some(project.scope.clone()),
        })?;
        for key in std::iter::once(&project.project_key).chain(&project.aliases) {
            // Transport variants within one binding are harmless; across bindings they are ambiguous.
            let canonical = canonical_project_key(key)?;
            if let Some(owner) = keys.get(&canonical) {
                anyhow::ensure!(
                    *owner == project.scope,
                    "duplicate canonical project identity"
                );
            } else {
                keys.insert(canonical, project.scope.clone());
            }
        }
        anyhow::ensure!(scopes.insert(&project.scope), "duplicate project binding");
        anyhow::ensure!(
            project.aliases.len() <= 64 && project.historical_scopes.len() <= 64,
            "too many project aliases"
        );
        for scope in &project.historical_scopes {
            anyhow::ensure!(
                !scope.starts_with("user:")
                    && !scope.is_empty()
                    && scope != &project.scope
                    && scopes.insert(scope),
                "invalid historical project Scope"
            );
        }
    }
    for registration in &state.registrations {
        anyhow::ensure!(
            registration.project_scopes.len() <= MAX_CLIENT_PROJECTS,
            "too many client projects"
        );
        let mut assigned = HashSet::new();
        for scope in &registration.project_scopes {
            anyhow::ensure!(
                assigned.insert(scope)
                    && state.projects.iter().any(|p| p.ready && &p.scope == scope),
                "client project grant lacks a ready binding"
            );
        }
        for scope in &registration.historical_project_scopes {
            anyhow::ensure!(
                state
                    .projects
                    .iter()
                    .any(|p| p.historical_scopes.contains(scope)
                        && registration.project_scopes.contains(&p.scope)),
                "historical grant lacks its canonical project"
            );
        }
    }
    Ok(())
}

impl EnrollmentHandler {
    pub(super) async fn set_project_registration_policy(
        &self,
        params: ProjectRegistrationPolicyParams,
    ) -> std::result::Result<(), ProtocolError> {
        let mut state = self.inner.state.lock().await;
        let mut next = state.clone();
        let registration = next
            .registrations
            .iter_mut()
            .find(|r| r.registration_id == params.registration_id && r.revoked_at.is_none())
            .ok_or_else(ProtocolError::not_found)?;
        if params.enabled
            && !matches!(
                registration.approved_access.mode,
                RequestedAccessMode::Contribute
                    | RequestedAccessMode::Write
                    | RequestedAccessMode::Admin
            )
        {
            return Err(ProtocolError::invalid(
                "project registration requires an approved contributing client",
            ));
        }
        registration.allow_project_registration = params.enabled;
        self.inner
            .state_file
            .write(&next)
            .map_err(|_| ProtocolError::internal())?;
        *state = next;
        Ok(())
    }

    pub(super) async fn ensure_project_scope(
        &self,
        mut params: EnsureProjectScopeParams,
    ) -> std::result::Result<EnrollmentResponse, ProtocolError> {
        validate_request(&params.project)
            .map_err(|_| ProtocolError::invalid("invalid project identity or metadata"))?;
        let requested_key = params.project.project_key.clone();
        params.project.project_key = canonical_project_key(&params.project.project_key)
            .map_err(|_| ProtocolError::invalid("invalid project identity or metadata"))?;
        let credential_hash = hash_credential(&params.credential)?;
        let result;
        {
            // Serializes binding, provisioning and grants, including concurrent identical requests.
            let mut state = self.inner.state.lock().await;
            let index = state
                .registrations
                .iter()
                .position(|r| {
                    r.registration_id == params.registration_id
                        && r.revoked_at.is_none()
                        && credential_matches(&r.credential_hash, &credential_hash)
                })
                .ok_or_else(ProtocolError::not_found)?;
            let registration = state.registrations[index].clone();
            let existing = state
                .projects
                .iter()
                .find(|p| {
                    std::iter::once(&p.project_key)
                        .chain(&p.aliases)
                        .any(|key| {
                            canonical_project_key(key).ok().as_ref()
                                == Some(&params.project.project_key)
                        })
                })
                .or_else(|| {
                    params
                        .project
                        .existing_scope
                        .as_ref()
                        .and_then(|scope| state.projects.iter().find(|p| &p.scope == scope))
                })
                .cloned();
            let already_granted = existing
                .as_ref()
                .is_some_and(|p| p.ready && registration.project_scopes.contains(&p.scope));
            if !already_granted && !registration.allow_project_registration {
                return Err(ProtocolError {
                    code: "project_registration_denied",
                    message: "project registration is not enabled for this client in PCP Console; no user-Scope fallback",
                });
            }
            let created = existing.is_none();
            let binding = if let Some(mut binding) = existing {
                if params
                    .project
                    .existing_scope
                    .as_ref()
                    .is_some_and(|s| s != &binding.scope)
                {
                    return Err(ProtocolError::invalid(
                        "projectKey is already bound to a different Scope",
                    ));
                }
                if !already_granted && binding.registration_id != registration.registration_id {
                    let access = access_session(
                        &registration,
                        &self.inner.identity_id,
                        &self.inner.service.generation,
                        Vec::new(),
                    )?;
                    if !access.allows(&binding.scope, AccessPermission::Ingest) {
                        return Err(ProtocolError::invalid(
                            "existing project requires its current write grant",
                        ));
                    }
                }
                if !std::iter::once(&binding.project_key)
                    .chain(&binding.aliases)
                    .any(|key| {
                        canonical_project_key(key).ok().as_ref()
                            == Some(&params.project.project_key)
                    })
                {
                    if binding.aliases.len() >= 64 {
                        return Err(ProtocolError::capacity());
                    }
                    binding.aliases.push(params.project.project_key.clone());
                }
                binding
            } else {
                if params.project.display_name.as_ref().is_some_and(|name| {
                    state
                        .projects
                        .iter()
                        .any(|p| p.display_name.eq_ignore_ascii_case(name))
                }) {
                    return Err(ProtocolError::invalid(
                        "project name is already registered; reuse its projectKey, explicitly adopt its writable existingScope, or distinguish an unrelated project name",
                    ));
                }
                if state.projects.len() >= MAX_PROJECTS
                    || registration.project_scopes.len() >= MAX_CLIENT_PROJECTS
                {
                    return Err(ProtocolError::capacity());
                }
                let name = params.project.display_name.clone().ok_or_else(|| {
                    ProtocolError::invalid(
                        "first register the project with projectKey and displayName",
                    )
                })?;
                let scopes = self
                    .inner
                    .store
                    .local_scope_names()
                    .await
                    .map_err(|_| ProtocolError::unavailable())?;
                let scope = if let Some(scope) = &params.project.existing_scope {
                    let access = access_session(
                        &registration,
                        &self.inner.identity_id,
                        &self.inner.service.generation,
                        Vec::new(),
                    )?;
                    if !scopes.contains(scope) || !access.allows(scope, AccessPermission::Ingest) {
                        return Err(ProtocolError::invalid(
                            "adopting an existing project requires its current write grant",
                        ));
                    }
                    scope.clone()
                } else {
                    let digest = format!(
                        "{:x}",
                        Sha256::digest(params.project.project_key.as_bytes())
                    );
                    let scope = format!("project:{}", &digest[..24]);
                    if scopes.contains(&scope) {
                        return Err(ProtocolError::invalid(
                            "project Scope collision requires operator review",
                        ));
                    }
                    scope
                };
                if state.projects.iter().any(|p| p.scope == scope) {
                    return Err(ProtocolError::invalid(
                        "Scope is already bound to another projectKey",
                    ));
                }
                let binding = StoredProject {
                    project_key: params.project.project_key.clone(),
                    aliases: Vec::new(),
                    historical_scopes: Vec::new(),
                    scope,
                    display_name: name,
                    description: params.project.description.clone(),
                    ready: false,
                    created_at: Utc::now(),
                    registration_id: registration.registration_id.clone(),
                };
                let mut next = state.clone();
                next.projects.push(binding.clone());
                self.inner
                    .state_file
                    .write(&next)
                    .map_err(|_| ProtocolError::internal())?;
                *state = next;
                binding
            };
            if !already_granted && registration.project_scopes.len() >= MAX_CLIENT_PROJECTS {
                return Err(ProtocolError::capacity());
            }
            if !binding.ready {
                let operator = AccessSession::full_control(
                    AccessPrincipal {
                        principal_id: "service:pcp-project-registration".into(),
                        principal_type: AccessPrincipalType::Service,
                        display_name: Some("PCP project registration".into()),
                    },
                    format!("project-registration:{}", self.inner.service.generation),
                    vec![binding.scope.clone()],
                );
                // Existing adopted Scopes retain their original display metadata.
                if !self
                    .inner
                    .store
                    .local_scope_names()
                    .await
                    .map_err(|_| ProtocolError::unavailable())?
                    .contains(&binding.scope)
                {
                    self.inner
                        .store
                        .create_scope(
                            &operator,
                            CreateScopeRequest {
                                namespace: binding.scope.clone(),
                                display_name: binding.display_name.clone(),
                                description: binding.description.clone(),
                                parent_namespace: None,
                            },
                        )
                        .await
                        .map_err(|_| ProtocolError::unavailable())?;
                }
            }
            let mut next = state.clone();
            let stored_binding = next
                .projects
                .iter_mut()
                .find(|p| p.project_key == binding.project_key)
                .unwrap();
            *stored_binding = binding.clone();
            stored_binding.ready = true;
            if !next.registrations[index]
                .project_scopes
                .contains(&binding.scope)
            {
                next.registrations[index]
                    .project_scopes
                    .push(binding.scope.clone());
            }
            for scope in &binding.historical_scopes {
                if !next.registrations[index]
                    .historical_project_scopes
                    .contains(scope)
                {
                    next.registrations[index]
                        .historical_project_scopes
                        .push(scope.clone());
                }
            }
            self.inner
                .state_file
                .write(&next)
                .map_err(|_| ProtocolError::internal())?;
            *state = next;
            result = ProjectScopeResult {
                project_key: requested_key,
                canonical_project_key: Some(binding.project_key),
                scope: binding.scope,
                display_name: binding.display_name,
                created,
            };
        }
        let session = self
            .open_registration(&params.registration_id, &credential_hash)
            .await?;
        Ok(EnrollmentResponse::new(
            "ensure_project_scope",
            EnrollmentResult::ProjectReady {
                project: result,
                session,
            },
        ))
    }
}
