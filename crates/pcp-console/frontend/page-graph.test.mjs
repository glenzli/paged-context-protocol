import assert from "node:assert/strict";
import test from "node:test";

import { relationFamily } from "../src/page-graph.js";

test("classifies source stream separately from stored graph edges", () => {
  assert.equal(relationFamily("source_precedes", "source_stream"), "source_stream");
  assert.equal(relationFamily("derived_from", "provenance"), "provenance");
  assert.equal(relationFamily("related_to", "relation"), "semantic");
});
