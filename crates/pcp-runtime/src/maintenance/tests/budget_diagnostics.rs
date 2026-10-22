use super::*;
use crate::maintenance::failure::{FailureKind, WorkerFailure};

struct RejectedWorker {
    budget: bool,
}
#[async_trait]
impl SemanticMaintenanceWorker for RejectedWorker {
    async fn evaluate(&self, _: MaintenanceWorkerRequest) -> Result<MaintenanceWorkerResponse> {
        if self.budget {
            Err(WorkerFailure::budget_wait().into())
        } else {
            Err(WorkerFailure::wait(
                FailureKind::ResponseTimeout,
                std::time::Duration::from_secs(120),
                std::time::Duration::from_secs(130),
                std::time::Duration::from_secs(130),
            )
            .into())
        }
    }
}

#[tokio::test]
async fn local_admission_wait_records_no_call_or_failure_but_unknown_execution_still_does() {
    for budget in [true, false] {
        let f = Fixture::open(if budget {
            "budget-denied"
        } else {
            "unknown-execution"
        })
        .await;
        let page = f
            .client
            .write_page(f.page("Original source remains untouched", "budget-source"))
            .await
            .unwrap();
        let mut m = RuntimeMaintainer::for_test(
            f.client.clone(),
            Arc::new(RejectedWorker { budget }),
            f.config(),
        );
        let mut report = MaintenanceCycleReport::default();
        let source = f
            .client
            .read_pages(pcp_core::ReadPagesRequest {
                page_ids: vec![page.page_id.clone()],
                revision_ids: vec![],
                projections: vec![Projection::Payload],
                max_chars: 1024,
            })
            .await
            .unwrap()
            .remove(0);
        let outcome = m
            .evaluate_isolated_worker(
                MaintenanceWorkerRequest::SummarizePage {
                    page: Box::new(source.into()),
                },
                "summary:source",
                vec![page.revision_id.clone()],
                &mut report,
            )
            .await;
        assert!(outcome.is_none());
        assert_eq!(report.worker_calls, u32::from(!budget));
        assert_eq!(report.isolated_jobs, u32::from(!budget));
        assert_eq!(report.deferred, 1);
        let health = f
            .client
            .health_snapshot(vec![f.namespace.clone()], 1)
            .await
            .unwrap();
        assert_eq!(health.model_usage.model_calls, u64::from(!budget));
        assert_eq!(
            f.client.current_revision_id(page.page_id).await.unwrap(),
            page.revision_id
        );
        f.close().await;
    }
}
