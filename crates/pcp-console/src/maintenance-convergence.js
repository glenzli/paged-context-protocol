const ADDITIVE_REPORT_FIELDS = [
  "workerCalls",
  "jobsAdvanced",
  "reconciliationsCommitted",
  "reconciliationsProposed",
  "summariesWritten",
  "summariesProposed",
  "packsCommitted",
  "packsProposed",
  "relationsCommitted",
  "relationsProposed",
  "retentionLeasesWritten",
  "retentionLeasesProposed",
  "topicsProposed",
  "archivesProposed",
  "reviewItemsProposed",
  "escalatedDecisions",
  "deferred",
];

export function mergeConvergenceReport(current, report) {
  if (!current) return { ...report };
  const merged = { ...current };
  merged.inspectedPages = Math.max(current.inspectedPages || 0, report.inspectedPages || 0);
  ADDITIVE_REPORT_FIELDS.forEach((key) => {
    merged[key] = (current[key] || 0) + (report[key] || 0);
  });
  return merged;
}

export function convergenceSettled(response) {
  return response?.settled === true && (response.report?.jobsAdvanced || 0) === 0;
}

export function convergencePhase(convergence, pendingReviewCount = 0) {
  if (convergence?.running) return "running";
  if (convergence?.error) return "failed";
  if (convergence?.completedAt && pendingReviewCount > 0) return "review";
  if (convergence?.completedAt) return "settled";
  return "waiting";
}

export function reconcileConvergenceStatus(convergence, {
  operationActive = false,
  automationState = "not_started",
  pendingReviewCount = 0,
  observedAt = null,
} = {}) {
  if (!convergence?.running) return convergence;
  if (operationActive || automationState === "running" || pendingReviewCount > 0) return convergence;
  return {
    ...convergence,
    running: false,
    completedAt: convergence.completedAt || observedAt,
    error: null,
  };
}
