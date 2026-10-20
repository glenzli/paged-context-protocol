import test from "node:test";
import assert from "node:assert/strict";
import { createModelCatalog, modelDeploymentLabel, modelDeploymentOptions, modelEffortsForDeployment } from "../src/model-catalog.js";

const model = (id, intents = { "reasoning.solve": "exceptional" }) => ({ deployment_id: id, supported_efforts: ["low", "high", "ultra"], intents, availability: "available", observation_stale: false });
test("new deployment versions appear automatically with exact intent grants and supported efforts", () => {
  const catalog = { available: true, models: [model("codex_gpt_6_1_sol"), model("future_luna", { "language.respond": "advanced" })] };
  assert.deepEqual(modelDeploymentOptions(catalog, "reasoning.solve").map(m => m.id), ["codex_gpt_6_1_sol"]);
  assert.deepEqual(modelDeploymentOptions(catalog, "language.respond").map(m => m.id), ["future_luna"]);
  assert.equal(modelDeploymentLabel("codex_gpt_6_1_sol"), "GPT-6.1 Sol");
  assert.deepEqual(modelEffortsForDeployment("codex_gpt_6_1_sol", catalog), { efforts: ["low", "high", "ultra"], verified: true });
  assert.equal(modelEffortsForDeployment("codex_gpt_6_1_sol", { ...catalog, stale: true }).verified, false);
});
test("refresh preserves configured selections after disappearance and marks failed observations", () => {
  const catalog = { available: true, models: [{ ...model("retired"), availability: "unavailable" }] };
  const options = modelDeploymentOptions(catalog, "reasoning.solve", "previous_custom");
  assert.equal(options.find(m => m.id === "retired").unavailable, true);
  assert.equal(options.find(m => m.id === "previous_custom").configured, true);
  assert.equal(modelDeploymentOptions({ models: [] }, "reasoning.solve", "previous_custom")[0].id, "previous_custom");
});
test("concurrent refreshes share one request and errors retain a marked previous snapshot", async () => {
  let calls = 0, release;
  const changes = [];
  const catalog = createModelCatalog({ request: async () => {
    calls++;
    if (calls > 1) throw new Error("offline");
    await new Promise(resolve => { release = resolve; });
    return { models: [model("codex_gpt_6_1_sol")], available: true, stale: false };
  }, changed: snapshot => changes.push(snapshot) });
  const first = catalog.refresh();
  const second = catalog.refresh();
  assert.equal(first, second);
  assert.equal(calls, 1);
  release(); await first;
  await catalog.refresh({ force: true });
  assert.equal(calls, 2);
  assert.equal(catalog.snapshot.models[0].deployment_id, "codex_gpt_6_1_sol");
  assert.equal(catalog.snapshot.stale, true);
  assert.equal(changes.length, 2);
});
