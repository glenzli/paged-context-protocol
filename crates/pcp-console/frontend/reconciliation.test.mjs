import test from "node:test";
import assert from "node:assert/strict";
import { reconciliationView } from "../src/maintenance-reconciliation.js";

test("feedback and discovered updates share exact old/new evidence projection", () => {
  const candidate = {target:{revisionId:"old",namespace:"symbiont"}, evidence:[{revisionId:"new",namespace:"user"}], replacement:{revisionId:"new"}};
  const view = reconciliationView(candidate);
  assert.equal(view.title, "Content update review");
  assert.equal(view.crossScope, true);
  assert.deepEqual(view.panels.map((panel) => panel.page.revisionId), ["old", "new"]);
  assert.equal(view.panels[0].label, "Earlier Page");
  assert.equal(view.panels[1].label, "Proposed replacement");
  const feedback = reconciliationView({...candidate, signal:{}});
  assert.equal(feedback.title, "Feedback reconciliation");
  assert.equal(feedback.panels[0].label, "Current evidence");
  assert.equal(reconciliationView({target:candidate.target,evidence:candidate.evidence}).panels[1].label, "Later related Page");
});

test("legacy proposals without replacement evidence are flagged", () => {
  assert.equal(reconciliationView({replacement:{revisionId:"missing"}}).replacementUnavailable, true);
  assert.equal(reconciliationView({target:{namespace:"one"},evidence:[{namespace:"one"}]}).crossScope, false);
});
