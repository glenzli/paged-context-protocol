//! Optional Runtime enrollment extension, not a Store-wide administration grant.
use serde::{Deserialize, Serialize};

/// Normalize repository transport spellings without guessing the owner of an opaque ID.
pub fn canonical_project_key(key: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !key.is_empty() && key.len() <= 240 && key == key.trim() && !key.starts_with('/'),
        "invalid project identity"
    );
    let lower = key.to_ascii_lowercase();
    let repository = if lower.starts_with("https://") {
        &key[8..]
    } else if lower.starts_with("http://") {
        &key[7..]
    } else if lower.starts_with("ssh://git@") {
        &key[10..]
    } else if lower.starts_with("git@") {
        let (host, path) = key[4..]
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid Git remote"))?;
        return canonical_project_key(&format!("https://{host}/{path}"));
    } else {
        anyhow::ensure!(
            !key.contains('@') && !key.contains("://"),
            "credentials and unsupported transports are not project identities"
        );
        key
    };
    anyhow::ensure!(
        repository
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/:".contains(&b)),
        "invalid project identity characters"
    );
    if let Some((host, path)) = repository.split_once('/') {
        if host.contains('.') {
            anyhow::ensure!(
                host.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
                "invalid repository host"
            );
            let path = path.trim_end_matches('/');
            let path = path.strip_suffix(".git").unwrap_or(path);
            let parts = path.split('/').collect::<Vec<_>>();
            anyhow::ensure!(
                parts
                    .iter()
                    .all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains(':')),
                "invalid repository path"
            );
            let host = host.to_ascii_lowercase();
            if host == "github.com" {
                anyhow::ensure!(
                    parts.len() == 2,
                    "GitHub identity must name an owner and repository, without a branch"
                );
                return Ok(format!("{host}/{}", path.to_ascii_lowercase()));
            }
            if host == "gitlab.com" {
                anyhow::ensure!(
                    parts.len() >= 2 && !parts.contains(&"-"),
                    "GitLab identity must name a repository, without a branch URL"
                );
            }
            return Ok(format!("{host}/{path}"));
        }
    }
    anyhow::ensure!(repository == key, "repository URL requires a host and path");
    Ok(key.to_owned())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectScopeRequest {
    /// Stable project identity, shared by branches/worktrees; not a directory basename.
    pub project_key: String,
    /// Required only when first registering this project.
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Explicit adoption of an existing project Scope already writable by this client.
    #[serde(default)]
    pub existing_scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScopeResult {
    /// Echo the requested identity for enrolled clients that attest this field exactly.
    pub project_key: String,
    /// Canonical binding identity; absent in older Runtime replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_project_key: Option<String>,
    pub scope: String,
    pub display_name: String,
    pub created: bool,
}

#[cfg(test)]
mod tests {
    use super::canonical_project_key;

    #[test]
    fn repository_transports_share_an_identity_and_opaque_ids_do_not_guess() {
        for key in [
            "github.com/Owner/Repo",
            "https://github.com/Owner/Repo.git/",
            "http://GITHUB.com/owner/repo",
            "git@github.com:Owner/Repo.git",
            "ssh://git@github.com/Owner/Repo",
        ] {
            assert_eq!(canonical_project_key(key).unwrap(), "github.com/owner/repo");
        }
        assert_eq!(
            canonical_project_key("https://gitlab.com/team/group/Repo.git").unwrap(),
            "gitlab.com/team/group/Repo"
        );
        for key in ["sites:project_123", "shadow", "example.test/wiki"] {
            assert_eq!(canonical_project_key(key).unwrap(), key);
        }
        for key in [
            "https://token@github.com/o/r",
            "ssh://token@github.com/o/r",
            "github.com/o/r/tree/main",
            "github.com/o/../r",
            "https://github.com/o/r?secret=x",
            "https://github.com/o/r#branch",
            "https://github.com/o/r%2Fother",
            " shadow",
        ] {
            assert!(canonical_project_key(key).is_err(), "{key}");
        }
    }
}
