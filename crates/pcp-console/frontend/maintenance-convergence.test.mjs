import test from "node:test";
import assert from "node:assert/strict";

import {
  convergencePhase,
  convergenceSettled,
  mergeConvergenceReport,
} from "../src/maintenance-convergence.js";

test("progressive convergence aggregates completed batches without summing inventory", () => {
  const merged = mergeConvergenceReport(
    { inspectedPages: 18, jobsAdvanced: 1, workerCalls: 1, packsCommitted: 1, reviewItemsProposed: 0 },
    { inspectedPages: 20, jobsAdvanced: 1, workerCalls: 2, packsCommitted: 0, reviewItemsProposed: 1, escalatedDecisions: 1 },
  );

  assert.equal(merged.inspectedPages, 20);
  assert.equal(merged.workerCalls, 3);
  assert.equal(merged.jobsAdvanced, 2);
  assert.equal(merged.packsCommitted, 1);
  assert.equal(merged.reviewItemsProposed, 1);
  assert.equal(merged.escalatedDecisions, 1);
});

test("convergence stops only on an explicit zero-call settled response", () => {
  assert.equal(convergenceSettled({ settled: true, report: { jobsAdvanced: 0, workerCalls: 0 } }), true);
  assert.equal(convergenceSettled({ settled: true, report: { jobsAdvanced: 1, workerCalls: 0 } }), false);
  assert.equal(convergenceSettled({ settled: false, report: { jobsAdvanced: 0, workerCalls: 0 } }), false);
});

test("convergence phase exposes waiting, running, review, settled, and failed states", () => {
  assert.equal(convergencePhase({ running: false }), "waiting");
  assert.equal(convergencePhase({ running: true, error: { message: "stale" } }), "running");
  assert.equal(convergencePhase({ running: false, completedAt: "2026-08-24T00:00:00Z" }, 2), "review");
  assert.equal(convergencePhase({ running: false, completedAt: "2026-08-24T00:00:00Z" }), "settled");
  assert.equal(convergencePhase({ running: false, error: { message: "failed" } }), "failed");
});
