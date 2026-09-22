import test from "node:test";
import assert from "node:assert/strict";
import { MODEL_ROUTE_GROUPS, modelEffortsForDeployment, resolveModelRouteGroups, summarizeModelRouteGroups } from "../src/model-route-groups.js";

test("known Infer deployments expose only their configured efforts", () => {
  const luna = modelEffortsForDeployment("codex_gpt_6_luna");
  assert.deepEqual(luna, { efforts: ["low", "medium", "high", "xhigh", "max"], verified: true });
  assert.equal(luna.efforts.includes("ultra"), false);
  assert.equal(luna.efforts.includes("none"), false);
  assert.equal(modelEffortsForDeployment("codex_gpt_6_sol").efforts.includes("ultra"), true);
  assert.equal(modelEffortsForDeployment("codex_gpt_5_6_luna").efforts.includes("ultra"), false);
  assert.deepEqual(modelEffortsForDeployment("custom_provider"), {
    efforts: ["none", "low", "medium", "high", "xhigh", "max", "ultra"], verified: false,
  });
});

test("category defaults preserve individual efforts and special review routing", () => {
  const original = Object.fromEntries(MODEL_ROUTE_GROUPS.flatMap((group) => group.operations.map((operation) => [operation, {
    deployment_id: "codex_gpt_6_luna", effort: "medium",
  }])));
  original.select_relation = { deployment_id: "codex_gpt_6_luna", effort: "high" };
  original.review_candidate_synthesis = { deployment_id: "legacy", effort: "medium" };
  const groups = summarizeModelRouteGroups(original);
  assert.deepEqual(resolveModelRouteGroups(original, groups), original);
  const knowledge = groups.find((group) => group.id === "knowledge");
  assert.deepEqual(knowledge.overrides.extract_topic, { deployment_id: "codex_gpt_6_luna", effort: "medium" });
  knowledge.defaultRoute = { deployment_id: "codex_gpt_6_sol", effort: "high" };
  const resolved = resolveModelRouteGroups(original, groups);
  assert.deepEqual(resolved.select_relation, { deployment_id: "codex_gpt_6_sol", effort: "high" });
  assert.deepEqual(resolved.extract_topic, original.extract_topic);
  assert.deepEqual(resolved.review_candidate_synthesis, original.review_candidate_synthesis);
});

test("category changes apply to operations without overrides", () => {
  const original = {
    summarize_page: { deployment_id: "old", effort: "medium" },
    summarize_pages: { deployment_id: "old", effort: "medium" },
  };
  const groups = summarizeModelRouteGroups(original);
  groups[0].defaultRoute = { deployment_id: "new", effort: "low" };
  const resolved = resolveModelRouteGroups(original, groups);
  assert.deepEqual(resolved.summarize_page, groups[0].defaultRoute);
  assert.deepEqual(resolved.summarize_pages, groups[0].defaultRoute);
});

test("saved category default does not flip when the first operation differs", () => {
  const routes = {
    select_relation: { deployment_id: "codex_gpt_6_luna", effort: "high" },
    extract_topic: { deployment_id: "codex_gpt_6_sol", effort: "high" },
  };
  const groupRoute = { deployment_id: "codex_gpt_6_sol", effort: "high" };
  const groups = summarizeModelRouteGroups(routes, { knowledge: groupRoute }, { select_relation: routes.select_relation });
  const knowledge = groups.find((group) => group.id === "knowledge");
  assert.deepEqual(knowledge.defaultRoute, groupRoute);
  assert.deepEqual(knowledge.overrides, { select_relation: routes.select_relation });
  assert.deepEqual(resolveModelRouteGroups(routes, groups), routes);
});
