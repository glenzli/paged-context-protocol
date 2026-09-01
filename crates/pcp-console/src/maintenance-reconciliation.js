// Shared exact-evidence projection for feedback and discovered updates.
export function reconciliationView(candidate = {}) {
  const evidence = candidate.evidence || [];
  const replacement = evidence.find((page) => page.revisionId === candidate.replacement?.revisionId);
  const target = candidate.target || {};
  const panels = [{label: "Current evidence", page: target}];
  if (replacement) panels.push({label: "Proposed replacement", page: replacement});
  for (const page of evidence) {
    if (page !== replacement) panels.push({label: "Correction evidence", page});
  }
  const scopes = new Set([target.namespace, candidate.feedback?.namespace, ...evidence.map((page) => page.namespace)].filter(Boolean));
  return {
    title: candidate.signal ? "Feedback reconciliation" : "Content update review",
    panels,
    crossScope: scopes.size > 1,
    replacementUnavailable: Boolean(candidate.replacement && !replacement),
  };
}
