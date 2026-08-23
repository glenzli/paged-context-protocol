const TERMINAL_BATCH_STATES = new Set(["completed", "failed"]);

export function beginBatch(batch) {
  batch.status = "running";
  batch.attempts = (batch.attempts || 0) + 1;
  batch.issue = null;
  return batch;
}

export function completeBatch(batch, values = {}) {
  Object.assign(batch, values);
  batch.status = "completed";
  batch.issue = null;
  return batch;
}

export function failBatch(batch, error, values = {}) {
  Object.assign(batch, values);
  batch.status = "failed";
  batch.issue = error?.message || String(error);
  return batch;
}

export function runnableBatchIndexes(batches, { retryFailed = false } = {}) {
  return batches
    .filter((batch) => retryFailed ? batch.status === "failed" : !TERMINAL_BATCH_STATES.has(batch.status))
    .map((batch) => batch.batchIndex);
}

export function batchProgress(batches = []) {
  const progress = {
    total: batches.length,
    processed: 0,
    completed: 0,
    failed: 0,
    running: 0,
    pending: 0,
  };
  for (const batch of batches) {
    if (batch.status in progress) progress[batch.status] += 1;
    if (TERMINAL_BATCH_STATES.has(batch.status)) progress.processed += 1;
  }
  return progress;
}
