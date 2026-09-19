use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod compiled {
    include!(concat!(env!("OUT_DIR"), "/build_identity.rs"));
}

/// Software release version; independent of PCP/MCP protocol and plugin package versions.
pub const SOFTWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Provenance compiled into this process, never inferred from the current checkout at runtime.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: String,
    pub git_revision: Option<String>,
    /// Whether the source inputs differed from HEAD. None means Git was unavailable.
    pub dirty: Option<bool>,
    /// SHA-256 of PCP workspace source inputs, not a hash of the executable or dependencies.
    pub source_digest: String,
    pub target: String,
}

impl BuildInfo {
    pub fn current() -> Self {
        Self {
            version: SOFTWARE_VERSION.into(),
            git_revision: compiled::GIT_REVISION.map(str::to_owned),
            dirty: compiled::DIRTY,
            source_digest: compiled::SOURCE_DIGEST.into(),
            target: compiled::TARGET.into(),
        }
    }

    pub fn label(&self) -> String {
        let revision = self.git_revision.as_deref().unwrap_or("unknown");
        let state = match self.dirty {
            Some(true) => "dirty",
            Some(false) => "clean",
            None => "unknown",
        };
        format!(
            "{} git:{} {} source:{}",
            self.version,
            &revision[..revision.len().min(12)],
            state,
            &self.source_digest[..self.source_digest.len().min(12)]
        )
    }
}
