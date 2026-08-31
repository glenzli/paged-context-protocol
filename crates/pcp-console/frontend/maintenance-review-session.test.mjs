import assert from "node:assert/strict";
import test from "node:test";

import {
  REVIEW_DECISION,
  partitionReviewSession,
  reconcileReviewDecisions,
  restoreReviewDecisions,
  reviewDecisionCounts,
  serializeReviewDecisions,
  stageReviewDecision,
  undoReviewDecision,
} from "../src/maintenance-review-session.js";

function review(candidateId, kind = "summary") {
  return { candidateId, payload: { kind, candidate: {} } };
}

test("review decisions remain reversible until the review session is committed", () => {
  const reviews = [review("summary-1"), review("relation-1", "relation")];
  const decisions = new Map();

  stageReviewDecision(decisions, reviews[0], REVIEW_DECISION.ACCEPT, "2026-08-23T13:00:00Z");
  stageReviewDecision(decisions, reviews[1], REVIEW_DECISION.SUPPRESS, "2026-08-23T13:00:01Z");

  assert.deepEqual(reviewDecisionCounts(decisions), {
    accept: 1,
    reject: 0,
    defer: 0,
    suppress: 1,
    total: 2,
  });
  assert.deepEqual(partitionReviewSession(reviews, decisions).pending, []);
  assert.equal(partitionReviewSession(reviews, decisions).staged.length, 2);

  assert.equal(undoReviewDecision(decisions, "summary-1"), true);
  assert.deepEqual(partitionReviewSession(reviews, decisions).pending, [reviews[0]]);
});

test("session persistence keeps only decisions that still match a pending review", () => {
  const decisions = new Map();
  stageReviewDecision(decisions, review("summary-1"), REVIEW_DECISION.REJECT);
  stageReviewDecision(decisions, review("relation-1", "relation"), REVIEW_DECISION.DEFER);

  const restored = restoreReviewDecisions(serializeReviewDecisions(decisions));
  reconcileReviewDecisions([review("relation-1", "relation")], restored);

  assert.equal(restored.has("summary-1"), false);
  assert.equal(restored.get("relation-1").decision, REVIEW_DECISION.DEFER);
});

test("permanent suppression is available only for relation reviews", () => {
  assert.throws(
    () => stageReviewDecision(new Map(), review("summary-1"), REVIEW_DECISION.SUPPRESS),
    /unsupported summary review decision/,
  );
});

test("feedback reconciliation uses the shared reversible review session", () => {
  const reconciliation = review("reconcile:feedback:target", "reconciliation");
  const decisions = new Map();

  stageReviewDecision(decisions, reconciliation, REVIEW_DECISION.ACCEPT);
  assert.equal(partitionReviewSession([reconciliation], decisions).staged.length, 1);
  assert.equal(undoReviewDecision(decisions, reconciliation.candidateId), true);
  assert.deepEqual(partitionReviewSession([reconciliation], decisions).pending, [reconciliation]);
  assert.throws(
    () => stageReviewDecision(decisions, reconciliation, REVIEW_DECISION.SUPPRESS),
    /unsupported reconciliation review decision/,
  );
});

test("malformed persisted review sessions fail closed", () => {
  assert.deepEqual([...restoreReviewDecisions("not-json")], []);
  assert.deepEqual([...restoreReviewDecisions(JSON.stringify([{ candidateId: "x", kind: "summary", decision: "erase" }]))], []);
});
