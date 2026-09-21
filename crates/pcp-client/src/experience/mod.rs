//! Optional experience evidence and delivery for native hosts. This is a Runtime
//! extension; records remain attributed observations, never executable policy.
pub mod outbox;
pub mod receipts;

use anyhow::{Result, ensure};
use pcp_core::SourceRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::context_hub::CandidateInput;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExperienceCandidate {
    pub candidate: CandidateInput,
    pub experience: Experience,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Experience {
    /// Producer-local subject; Runtime binds it to the submitting principal and Scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<String>,
    pub attempt: String,
    pub observation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipts: Vec<ExecutionReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub source: SourceRef,
    pub stage: ReceiptStage,
    pub outcome: ReceiptOutcome,
    /// A bounded snapshot of what the producer reported, not a trust assertion.
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStage {
    Inference,
    Validation,
    Publication,
    Task,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    Succeeded,
    Failed,
    Unknown,
}

pub(crate) fn bounded(name: &str, value: &str, limit: usize) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{name} cannot be empty");
    ensure!(
        value.chars().count() <= limit,
        "{name} exceeds {limit} characters"
    );
    Ok(())
}

impl Experience {
    pub fn validate(&self) -> Result<()> {
        if let Some(key) = &self.topic_key {
            bounded("experience topicKey", key, 120)?;
        }
        if let Some(text) = &self.conditions {
            bounded("experience conditions", text, 600)?;
        }
        bounded("experience attempt", &self.attempt, 600)?;
        bounded("experience observation", &self.observation, 1000)?;
        if let Some(text) = &self.interpretation {
            bounded("experience interpretation", text, 600)?;
        }
        ensure!(self.unresolved.len() <= 4, "too many experience questions");
        for text in &self.unresolved {
            bounded("experience question", text, 200)?;
        }
        ensure!(self.receipts.len() <= 4, "too many experience receipts");
        for receipt in &self.receipts {
            bounded("receipt summary", &receipt.summary, 600)?;
            bounded("receipt provider", &receipt.source.provider_id, 120)?;
            bounded("receipt locator", &receipt.source.locator, 1000)?;
            if let Some(version) = &receipt.version {
                bounded("receipt version", version, 160)?;
            }
            if let Some(digest) = &receipt.source.content_digest {
                bounded("receipt digest", digest, 160)?;
            }
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= 12_000,
            "experience metadata exceeds budget"
        );
        ensure!(
            serde_json::to_string(self)?.chars().count() <= 6000,
            "experience exceeds the organization context budget"
        );
        Ok(())
    }

    /// Use a short, human-readable account; preserve the structured evidence separately.
    pub fn into_candidate(self, scope: String, title: String) -> Result<ExperienceCandidate> {
        self.validate()?;
        bounded("scope", &scope, 256)?;
        bounded("title", &title, 120)?;
        let mut source_refs = Vec::new();
        for receipt in &self.receipts {
            if !source_refs
                .iter()
                .any(|s| serde_json::to_value(s).ok() == serde_json::to_value(&receipt.source).ok())
            {
                source_refs.push(receipt.source.clone());
            }
        }
        let mut input = CandidateInput {
            scope,
            event_id: String::new(),
            title,
            content: format!("{}\n\n{}", self.attempt, self.observation),
            source_refs,
            based_on_revision_ids: Vec::new(),
        };
        input.event_id = format!(
            "experience:{:x}",
            Sha256::digest(serde_json::to_vec(&(&input, &self))?)
        );
        Ok(ExperienceCandidate {
            candidate: input,
            experience: self,
        })
    }
}
