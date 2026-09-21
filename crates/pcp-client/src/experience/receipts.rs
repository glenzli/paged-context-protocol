//! Field-whitelisted adapters. A completed inference or publication is not a
//! successful task. Raw prompts, credentials and transcripts are never copied.
use anyhow::{Result, ensure};
use pcp_core::SourceRef;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{ExecutionReceipt, ReceiptOutcome, ReceiptStage, bounded};

fn source(bytes: &[u8], provider: &str, locator: String) -> Result<SourceRef> {
    ensure!(bytes.len() <= 1024 * 1024, "receipt exceeds 1 MiB");
    bounded("receipt locator", &locator, 1000)?;
    Ok(SourceRef {
        provider_id: provider.into(),
        locator,
        media_type: Some("application/json".into()),
        content_digest: Some(format!("sha256:{:x}", Sha256::digest(bytes))),
    })
}

pub fn infer_response(bytes: &[u8]) -> Result<ExecutionReceipt> {
    ensure!(bytes.len() <= 1024 * 1024, "receipt exceeds 1 MiB");
    #[derive(Deserialize)]
    struct Response {
        id: String,
        model: String,
        status: String,
    }
    let response: Response = serde_json::from_slice(bytes)?;
    let mut receipt = infer_execution(&response.id, &response.model, &response.status)?;
    receipt.source.content_digest = Some(format!("sha256:{:x}", Sha256::digest(bytes)));
    Ok(receipt)
}

pub fn infer_execution(id: &str, model: &str, status: &str) -> Result<ExecutionReceipt> {
    bounded("response id", id, 160)?;
    bounded("response model", model, 160)?;
    bounded("response status", status, 80)?;
    let outcome = match status {
        "completed" => ReceiptOutcome::Succeeded,
        "failed" | "cancelled" | "incomplete" => ReceiptOutcome::Failed,
        _ => ReceiptOutcome::Unknown,
    };
    Ok(ExecutionReceipt {
        source: SourceRef {
            provider_id: "infer-runtime".into(),
            locator: format!("response:{id}"),
            media_type: Some("application/json".into()),
            content_digest: None,
        },
        stage: ReceiptStage::Inference,
        outcome,
        summary: format!(
            "Inference {status} using {model}. This reports execution, not task success."
        ),
        version: None,
    })
}

/// `locator` belongs to the native host's durable receipt archive. PCP does not
/// open it or imply that the archive will survive the host's retention policy.
pub fn dev_mesh_commit(bytes: &[u8], locator: String) -> Result<ExecutionReceipt> {
    let source = source(bytes, "dev-mesh", locator)?;
    #[derive(Deserialize)]
    struct Commit {
        protocol: String,
        protocol_version: String,
        status: String,
        candidate_revision: Option<String>,
    }
    let commit: Commit = serde_json::from_slice(bytes)?;
    ensure!(
        commit.protocol == "dev-mesh.coordination" && commit.protocol_version == "20260823.1",
        "unsupported Dev Mesh receipt contract"
    );
    let outcome = if commit.status == "completed" {
        let revision = commit.candidate_revision.as_deref().unwrap_or_default();
        ensure!(
            [40, 64].contains(&revision.len()) && revision.bytes().all(|c| c.is_ascii_hexdigit()),
            "completed publication has no exact revision"
        );
        ReceiptOutcome::Succeeded
    } else {
        ReceiptOutcome::Unknown
    };
    bounded("publication status", &commit.status, 80)?;
    Ok(ExecutionReceipt {
        source,
        stage: ReceiptStage::Publication,
        outcome,
        summary: format!(
            "Publication status: {}. Validation evidence in the source is producer-reported; publication alone does not verify behavior.",
            commit.status
        ),
        version: commit.candidate_revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapters_keep_execution_separate_and_never_copy_raw_content() {
        let bytes = br#"{"id":"resp-1","model":"m","status":"completed","output":"PRIVATE PROMPT","token":"SECRET"}"#;
        let receipt = infer_response(bytes).unwrap();
        let encoded = serde_json::to_string(&receipt).unwrap();
        assert!(!encoded.contains("PRIVATE") && !encoded.contains("SECRET"));
        assert!(encoded.contains("inference") && encoded.contains("not task success"));
        assert!(
            receipt
                .source
                .content_digest
                .unwrap()
                .starts_with("sha256:")
        );
        let unknown = infer_response(br#"{"id":"r","model":"m","status":"queued"}"#).unwrap();
        assert!(matches!(unknown.outcome, ReceiptOutcome::Unknown));
    }
    #[test]
    fn publication_needs_a_revision_and_does_not_assert_tests_passed() {
        let bytes = br#"{"protocol":"dev-mesh.coordination","protocol_version":"20260823.1","status":"completed","candidate_revision":"3507527be8ec5692c3a5380298d4c815d39d5d76","validation_evidence":"all passed"}"#;
        let receipt = dev_mesh_commit(bytes, "archive:commit-1".into()).unwrap();
        assert!(matches!(receipt.stage, ReceiptStage::Publication));
        assert!(!receipt.summary.contains("all passed"));
        assert!(dev_mesh_commit(br#"{"protocol":"dev-mesh.coordination","protocol_version":"20260823.1","status":"completed"}"#, "a".into()).is_err());
    }
}
