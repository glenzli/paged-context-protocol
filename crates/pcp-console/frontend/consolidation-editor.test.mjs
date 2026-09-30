import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../src/app.js", import.meta.url), "utf8");
function code(start, end) {
  const from = source.indexOf(start), to = source.indexOf(end, from + start.length);
  assert.ok(from >= 0 && to > from);
  return source.slice(from, to);
}
const response = (status, body) => ({ ok: status < 400, status, statusText: "error", json: async () => body });

function editor(replies = [], language = "zh") {
  const calls = [], document = { activeElement: null };
  class Node {
    constructor(tag, className = "", text = "") {
      Object.assign(this, { tag, className, textContent: text, children: [], listeners: {}, disabled: false });
    }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
    addEventListener(event, callback) { this.listeners[event] = callback; }
    querySelectorAll(selector) {
      const tags = selector.split(",").map(s => s.trim());
      return this.children.flatMap(child => [...(tags.includes(child.tag) ? [child] : []), ...child.querySelectorAll(selector)]);
    }
    focus() { document.activeElement = this; }
  }
  const state = { maintenance: { consolidationDrafts: new Map() } };
  const context = {
    state, currentLanguage: language, document,
    element: (tag, cls, text) => new Node(tag, cls, text),
    fetch: async (path, options) => {
      calls.push({ path, body: JSON.parse(options.body) });
      const next = replies.shift();
      if (next instanceof Error) throw next;
      return typeof next === "function" ? next() : next || response(200, {});
    },
    loadRelationReviews: async () => {}, loadOverview: async () => {},
    showError: () => {}, pageInspector: { open: () => {} },
  };
  runInNewContext(code("async function api(", "async function enrollmentMutation("), context);
  runInNewContext(code("async function maintenanceMutation(", "async function governanceMutation("), context);
  runInNewContext(code("function consolidationReviewEditor(", "function maintenanceReviewCard("), context);
  const review = { candidateId: "review:example", proposedAt: "2026-09-29", payload: { candidate: {
    target: { pageId: "old-1", revisionId: "rev-1", namespace: "project:example", content: "First source" },
    evidence: [{ pageId: "old-2", revisionId: "rev-2", namespace: "project:example", content: "Second source" }],
    suggestedConsolidation: {
      outputs: [{ title: "Combined title", content: "Complete content", sourceIndexes: [0, 1] }],
      coverage: [0, 1].map(sourceIndex => ({ sourceIndex, complete: true, explanation: "Claims and limits retained" })),
    },
  } } };
  const tree = context.consolidationReviewEditor(review);
  const nodes = () => tree.querySelectorAll("input, textarea, button");
  const checks = () => nodes().filter(n => n.tag === "input" && n.type === "checkbox");
  const publish = () => nodes().find(n => /发布融合页面|重试同一方案|Publish consolidated|Retry same/.test(n.textContent));
  const confirm = () => { const c = checks()[0]; c.checked = true; c.listeners.change(); };
  return { calls, context, tree, nodes, checks, publish, confirm, state, document,
    draft: () => state.maintenance.consolidationDrafts.get(review.candidateId) };
}

const rejection = () => response(422, { error: "Review source changed", code: "consolidation_validation_failed", mutationOutcome: "not_applied" });

test("source membership and model suggestions do not count as human coverage confirmation", async () => {
  for (const language of ["zh", "en"]) {
    const ui = editor([], language);
    assert.deepEqual(ui.checks().map(c => c.checked), [false, false, true, true]);
    await ui.publish().listeners.click();
    assert.equal(ui.calls.length, 0);
    assert.equal(ui.draft().submitted, false);
    assert.ok(ui.nodes().every(n => !n.disabled));
    assert.equal(ui.document.activeElement, ui.checks()[0]);
    assert.match(ui.draft().error, language === "zh" ? /至少勾选一项/ : /at least one source/);
  }
});

test("explicit pre-write rejection preserves edited content and reopens the draft", async () => {
  const ui = editor([rejection(), response(200, {})]);
  const title = ui.nodes().find(n => n.tag === "input" && n.type !== "checkbox");
  title.value = "My reviewed title"; title.listeners.input();
  ui.confirm(); await ui.publish().listeners.click();
  assert.equal(ui.draft().submitted, false);
  assert.equal(ui.draft().failed, false);
  assert.equal(ui.draft().outputs[0].title, "My reviewed title");
  assert.ok(ui.nodes().every(n => !n.disabled));
  assert.match(ui.draft().error, /方案未发布，可修改/);
  await ui.publish().listeners.click();
  assert.equal(ui.calls.length, 2);
  assert.equal(ui.draft(), undefined);
});

test("lost or unclassified responses freeze the exact plan, even if a later retry is rejected", async () => {
  for (const failure of [new Error("connection lost"), response(500, { error: "ledger error" }),
    { ok: true, status: 200, json: async () => { throw new Error("truncated response"); } }]) {
    const ui = editor([failure, rejection(), response(200, {})]);
    ui.confirm(); await ui.publish().listeners.click();
    assert.equal(ui.draft().uncertain, true);
    assert.ok(ui.nodes().filter(n => n.tag !== "button").every(n => n.disabled));
    assert.equal(ui.publish().disabled, false);
    await ui.publish().listeners.click();
    assert.equal(ui.draft().submitted, true);
    assert.equal(ui.draft().uncertain, true);
    assert.ok(ui.nodes().filter(n => n.tag !== "button").every(n => n.disabled));
    await ui.publish().listeners.click();
    assert.equal(ui.calls.length, 3);
    assert.deepEqual(ui.calls[0], ui.calls[1]);
    assert.deepEqual(ui.calls[1], ui.calls[2]);
    assert.equal(ui.draft(), undefined);
  }
});

test("a generic HTTP error cannot grant edit recovery just by including a marker", async () => {
  const ui = editor([response(500, { error: "unknown", code: "consolidation_validation_failed", mutationOutcome: "not_applied" })]);
  ui.confirm(); await ui.publish().listeners.click();
  assert.equal(ui.draft().uncertain, true);
  assert.equal(ui.draft().submitted, true);
});

test("double submit is blocked while a consolidation is in flight", async () => {
  let finish;
  const ui = editor([() => new Promise(resolve => { finish = resolve; })]);
  ui.confirm(); const button = ui.publish();
  const pending = button.listeners.click();
  await button.listeners.click();
  assert.equal(ui.calls.length, 1);
  finish(response(200, {})); await pending;
  assert.equal(ui.draft(), undefined);
});
