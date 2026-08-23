import assert from "node:assert/strict";
import test from "node:test";

import {
  RELATION_DECISION,
  bulkSelectableCandidates,
  partitionMaintenanceDecisions,
  stageRelationDecision,
  toggleBulkSelection,
} from "../src/maintenance-relation-decisions.js";

test("bulk selection never overwrites explicit relation decisions", () => {
  const summary = { candidateId: "summary-1", operation: "summary" };
  const rejected = { candidateId: "relation-rejected", operation: "relation" };
  const skipped = { candidateId: "relation-skipped", operation: "relation" };
  const candidates = [summary, rejected, skipped];
  const selected = new Set([summary.candidateId]);
  const decisions = new Map();
  const draftStates = new Map();

  stageRelationDecision(selected, decisions, rejected.candidateId, RELATION_DECISION.REJECT);
  stageRelationDecision(selected, decisions, skipped.candidateId, RELATION_DECISION.SKIP);
  const bulkCandidates = bulkSelectableCandidates(candidates, draftStates);
  toggleBulkSelection(bulkCandidates, selected);
  toggleBulkSelection(bulkCandidates, selected);

  assert.equal(decisions.get(rejected.candidateId), RELATION_DECISION.REJECT);
  assert.equal(decisions.get(skipped.candidateId), RELATION_DECISION.SKIP);
  assert.equal(selected.has(rejected.candidateId), true);
  assert.equal(selected.has(skipped.candidateId), false);
  assert.deepEqual(
    partitionMaintenanceDecisions(candidates, selected, decisions, draftStates),
    { candidates: [summary], rejections: [rejected], suppressions: [] },
  );
});

test("accept reject skip and suppress partition into distinct apply paths", () => {
  const accepted = { candidateId: "accepted", operation: "relation" };
  const rejected = { candidateId: "rejected", operation: "relation" };
  const skipped = { candidateId: "skipped", operation: "relation" };
  const suppressed = { candidateId: "suppressed", operation: "relation" };
  const candidates = [accepted, rejected, skipped, suppressed];
  const selected = new Set();
  const decisions = new Map();
  const draftStates = new Map([[suppressed.candidateId, "suppressed"]]);

  stageRelationDecision(selected, decisions, accepted.candidateId, RELATION_DECISION.ACCEPT);
  stageRelationDecision(selected, decisions, rejected.candidateId, RELATION_DECISION.REJECT);
  stageRelationDecision(selected, decisions, skipped.candidateId, RELATION_DECISION.SKIP);

  assert.deepEqual(
    partitionMaintenanceDecisions(candidates, selected, decisions, draftStates),
    { candidates: [accepted], rejections: [rejected], suppressions: [suppressed] },
  );
});
