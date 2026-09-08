export const REVIEW_DECISION = Object.freeze({
  ACCEPT: "accept",
  REJECT: "reject",
  DEFER: "defer",
  SUPPRESS: "suppress",
});

const REVIEW_DECISIONS = new Set(Object.values(REVIEW_DECISION));

export function reviewKind(review) {
  return review?.payload?.kind || "unknown";
}

function reviewSnapshot(review) {
  const candidate = review?.payload?.candidate || {};
  // Persist identity only, not source content. A re-analyzed candidate must
  // not inherit a previous staged approval merely because its ID is stable.
  return JSON.stringify([
    review?.proposedAt ?? null, review?.updatedAt ?? null,
    candidate.expectedAssessmentRevisionId ?? null,
    candidate.target?.revisionId ?? null, candidate.replacement?.revisionId ?? null,
    candidate.disposition ?? null, candidate.basisRevisionIds ?? [],
  ]);
}

export function canStageReviewDecision(review, decision) {
  return REVIEW_DECISIONS.has(decision)
    && (decision !== REVIEW_DECISION.SUPPRESS || reviewKind(review) === "relation");
}

export function stageReviewDecision(decisions, review, decision, stagedAt = new Date().toISOString()) {
  if (!canStageReviewDecision(review, decision)) {
    throw new Error(`unsupported ${reviewKind(review)} review decision: ${decision}`);
  }
  decisions.set(review.candidateId, {
    candidateId: review.candidateId,
    kind: reviewKind(review),
    snapshot: reviewSnapshot(review),
    decision,
    stagedAt,
    error: null,
  });
  return decisions.get(review.candidateId);
}

export function undoReviewDecision(decisions, candidateId) {
  return decisions.delete(candidateId);
}

export function reconcileReviewDecisions(reviews, decisions) {
  const reviewsById = new Map(reviews.map((review) => [review.candidateId, review]));
  for (const [candidateId, staged] of decisions) {
    const review = reviewsById.get(candidateId);
    if (!review || staged.kind !== reviewKind(review) || staged.snapshot !== reviewSnapshot(review) || !canStageReviewDecision(review, staged.decision)) {
      decisions.delete(candidateId);
    }
  }
  return decisions;
}

export function partitionReviewSession(reviews, decisions) {
  reconcileReviewDecisions(reviews, decisions);
  const pending = [];
  const staged = [];
  for (const review of reviews) {
    const decision = decisions.get(review.candidateId);
    if (decision) staged.push({ review, decision });
    else pending.push(review);
  }
  return { pending, staged };
}

export function reviewDecisionCounts(decisions) {
  const counts = { accept: 0, reject: 0, defer: 0, suppress: 0, total: 0 };
  for (const staged of decisions.values()) {
    if (Object.hasOwn(counts, staged.decision)) counts[staged.decision] += 1;
    counts.total += 1;
  }
  return counts;
}

export function serializeReviewDecisions(decisions) {
  return JSON.stringify([...decisions.values()].map(({ candidateId, kind, decision, stagedAt, snapshot, reason }) => ({
    candidateId,
    kind,
    decision,
    stagedAt,
    snapshot,
    reason,
  })));
}

export function restoreReviewDecisions(serialized) {
  if (!serialized) return new Map();
  try {
    const entries = JSON.parse(serialized);
    if (!Array.isArray(entries)) return new Map();
    return new Map(entries.flatMap((entry) => (
      entry
        && typeof entry.candidateId === "string"
        && typeof entry.kind === "string"
        && REVIEW_DECISIONS.has(entry.decision)
        ? [[entry.candidateId, { ...entry, error: null }]]
        : []
    )));
  } catch (_) {
    return new Map();
  }
}

// Group for presentation only: every alternative retains its own explicit decision.
export function groupReviewTopics(reviews) {
  const groups = [];
  const terms = (title) => {
    const text = String(title || "").toLowerCase().replace(/[\s\p{P}\p{S}]/gu, "");
    return new Set(Array.from({ length: Math.max(0, text.length - 2) }, (_, i) => text.slice(i, i + 3)));
  };
  const related = (a, b) => {
    const ca = a.payload.candidate, cb = b.payload.candidate;
    if (ca.refreshTarget?.pageId && ca.refreshTarget.pageId === cb.refreshTarget?.pageId) return true;
    const sources = new Set((ca.pages || []).map((p) => p.pageId));
    if ((cb.pages || []).filter((p) => sources.has(p.pageId)).length >= 2) return true;
    const left = terms(ca.title), right = terms(cb.title);
    const shared = [...left].filter((t) => right.has(t)).length;
    return shared >= 4 && shared / Math.max(1, left.size + right.size - shared) >= 0.55;
  };
  for (const review of reviews) {
    const group = reviewKind(review) === "topic"
      ? groups.find((g) => reviewKind(g[0]) === "topic" && related(g[0], review)) : null;
    if (group) group.push(review);
    else groups.push([review]);
  }
  return groups;
}
