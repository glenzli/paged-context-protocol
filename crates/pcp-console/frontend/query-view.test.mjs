import assert from "node:assert/strict";
import test from "node:test";

import { buildQueryRequest } from "../src/query_view.js";

test("semantic retrieval omits Router-only effort instead of serializing null", () => {
  assert.deepEqual(
    buildQueryRequest({
      method: "semantic_search",
      query: "PCP",
      scope: "",
      topK: "6",
      intentEffort: "medium",
    }),
    { query: "PCP", scopes: [], resultLimit: 6 },
  );
});

test("intent matching carries its selected Router effort", () => {
  assert.deepEqual(
    buildQueryRequest({
      method: "match_intent",
      query: "蒸馏的安全边界",
      scope: "project:symbiont-d",
      topK: 12,
      intentEffort: "high",
    }),
    {
      query: "蒸馏的安全边界",
      scopes: ["project:symbiont-d"],
      resultLimit: 12,
      intentEffort: "high",
    },
  );
});
