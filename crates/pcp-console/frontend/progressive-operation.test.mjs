import assert from "node:assert/strict";
import test from "node:test";

import {
  batchProgress,
  beginBatch,
  completeBatch,
  failBatch,
  runnableBatchIndexes,
} from "../src/progressive-operation.js";

test("batch lifecycle keeps attempts and terminal progress explicit", () => {
  const pending = { batchIndex: 0, status: "pending", attempts: 0 };
  const failed = { batchIndex: 1, status: "failed", attempts: 1, issue: "bad response" };

  beginBatch(pending);
  assert.equal(pending.status, "running");
  assert.equal(pending.attempts, 1);
  completeBatch(pending, { candidateIds: ["candidate:1"] });
  beginBatch(failed);
  failBatch(failed, new Error("still bad"));

  assert.deepEqual(batchProgress([pending, failed]), {
    total: 2,
    processed: 2,
    completed: 1,
    failed: 1,
    running: 0,
    pending: 0,
  });
  assert.equal(failed.attempts, 2);
  assert.equal(failed.issue, "still bad");
});

test("normal runs skip terminal batches while retry runs only failures", () => {
  const batches = [
    { batchIndex: 0, status: "completed" },
    { batchIndex: 1, status: "failed" },
    { batchIndex: 2, status: "pending" },
    { batchIndex: 3, status: "running" },
  ];
  assert.deepEqual(runnableBatchIndexes(batches), [2, 3]);
  assert.deepEqual(runnableBatchIndexes(batches, { retryFailed: true }), [1]);
});
