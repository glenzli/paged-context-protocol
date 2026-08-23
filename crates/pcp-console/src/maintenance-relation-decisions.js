export const RELATION_DECISION = Object.freeze({
  ACCEPT: "accepted",
  REJECT: "rejected",
  SKIP: "skipped",
});

export function stageRelationDecision(selected, decisions, candidateId, decision) {
  if (!Object.values(RELATION_DECISION).includes(decision)) {
    throw new Error(`unsupported relation decision: ${decision}`);
  }
  if (decision === RELATION_DECISION.SKIP) selected.delete(candidateId);
  else selected.add(candidateId);
  decisions.set(candidateId, decision);
}

export function bulkSelectableCandidates(candidates, draftStates) {
  return candidates.filter((candidate) => (
    candidate.operation !== "relation" && draftStates.get(candidate.candidateId) !== "suppressed"
  ));
}

export function toggleBulkSelection(candidates, selected) {
  const allSelected = candidates.length > 0
    && candidates.every((candidate) => selected.has(candidate.candidateId));
  for (const candidate of candidates) selected.delete(candidate.candidateId);
  if (!allSelected) {
    for (const candidate of candidates) selected.add(candidate.candidateId);
  }
}

export function partitionMaintenanceDecisions(candidates, selected, decisions, draftStates) {
  const suppressions = candidates.filter((candidate) => (
    candidate.operation === "relation" && draftStates.get(candidate.candidateId) === "suppressed"
  ));
  const suppressionIds = new Set(suppressions.map((candidate) => candidate.candidateId));
  const rejections = candidates.filter((candidate) => (
    candidate.operation === "relation"
      && decisions.get(candidate.candidateId) === RELATION_DECISION.REJECT
      && !suppressionIds.has(candidate.candidateId)
  ));
  const applicable = candidates.filter((candidate) => {
    if (suppressionIds.has(candidate.candidateId)) return false;
    if (candidate.operation === "relation") {
      return decisions.get(candidate.candidateId) === RELATION_DECISION.ACCEPT;
    }
    return selected.has(candidate.candidateId);
  });
  return { candidates: applicable, rejections, suppressions };
}
