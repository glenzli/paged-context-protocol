import test from "node:test";
import assert from "node:assert/strict";
import { createAccessView } from "../src/access-view.js";

class Node {
  constructor(text = "") { this.textContent = text; this.children = []; this.value = ""; this.checked = false; this.handlers = {}; this.attributes = {}; }
  append(...nodes) { this.children.push(...nodes); }
  prepend(...nodes) { this.children.unshift(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(key, value) { this.attributes[key] = value; }
  addEventListener(name, handler) { this.handlers[name] = handler; }
}
function fixture(api, options = {}) {
  const nodes = new Map();
  const byId = id => { if (!nodes.has(id)) nodes.set(id, new Node()); return nodes.get(id); };
  globalThis.document = { createElement: () => new Node() };
  const view = createAccessView({ api, byId, element: (tag, className, text) => Object.assign(new Node(text), {tag,className}), t: value => value, formatTime: value => value, formatNumber: String, showError: error => { throw error; }, ...options });
  return { view, byId };
}
const event = id => ({ eventId: id, principal: { principalId: "client:a" }, sessionId: "session:a", operation: "read_pages", occurredAt: "2026-09-08T08:00:00Z", scopes: ["a"], decision: "allowed" });
const response = (id, cursor = null) => ({ events: [event(id)], nextCursor: cursor, totalEvents: 1234, clients: [{ principal: { principalId: "client:a", displayName: "Client A" }, eventCount: 1234, lastAccessAt: "2026-09-08T08:00:00Z" }], operations: [{ operation: "read_pages", eventCount: 1234 }] });
const text = node => [node.textContent, ...node.children.map(text)].join("");

test("audit rows omit repeated scopes while details keep every historical name and identifier", async () => {
  const scopes = ["project:one", "project:retired", "project:three", "project:missing"];
  const names = {"project:one":"Project One", "project:retired":"Historical project", "project:three":"Project Three"};
  const { view, byId } = fixture(async () => {
    const result = response("long");
    result.events[0].scopes = scopes; result.scopeNames = names;
    return result;
  });
  await view.load();
  const rows = byId("access-rows").children;
  assert.equal(rows[0].children.length, 5);
  assert.doesNotMatch(text(rows[0]), /Project One|Historical project|project:/);
  const details = rows[1].children[0].children[0];
  assert.equal(details.tag, "details");
  assert.ok(!details.open);
  assert.equal(text(details.children[0]), "Recorded request scopes · 4 scopes in total");
  const items = details.children[1].children;
  assert.deepEqual(items.map(item => item.children[0].textContent), ["Project One", "Historical project", "Project Three", "Unknown scope"]);
  assert.deepEqual(items.map(item => item.children[1].textContent), scopes);
});

test("pagination keeps expanded rows and preserves historical names in request details", async () => {
  let calls = 0;
  const { view, byId } = fixture(async () => {
    const result = response(String(++calls), calls === 1 ? "next-page" : null);
    result.events[0].scopes = calls === 1 ? ["project:retired"] : ["project:current", "project:new"];
    result.scopeNames = calls === 1 ? {"project:retired":"Historical project"} : {"project:current":"Old current name", "project:new":"New project"};
    return result;
  }, {scopeName: scope => scope === "project:current" ? "Current project" : scope});
  await view.load();
  const firstRow = byId("access-rows").children[0], detail = firstRow.children[4].children[0];
  detail.open = true; detail.handlers.toggle();
  await view.load({append:true});
  const rows = byId("access-rows").children;
  assert.equal(rows[0], firstRow);
  assert.equal(detail.open, true);
  assert.equal(rows[1].hidden, false);
  assert.match(text(rows[1]), /Historical project/);
  assert.match(text(rows[3]), /Current project/);
  assert.match(text(rows[3]), /New project/);
});

test("session permission fallback is explained without claiming per-project reads", async () => {
  const {view, byId} = fixture(async () => {
    const result = response("session");
    result.events[0].telemetry = {durationMs:0, request:{root:true, internalOperations:0}};
    return result;
  });
  await view.load();
  const scopeDetails = byId("access-rows").children[1].children[0].children[0];
  assert.match(text(scopeDetails.children[0]), /Session scopes/);
  assert.match(text(scopeDetails), /not evidence that every project was read/);
});

test("operation choices use readable labels, keep other choices after filtering, and reset all filters", async () => {
  const calls = [];
  const operations = ["read_pages","capture","publish_activity","read_activity","semantic_search","consolidate_pages","submit_feedback"];
  const {view, byId} = fixture(async url => {
    const parsed = new URL(url,"http://localhost"); calls.push(parsed);
    return {...response(String(calls.length)), operations:(parsed.searchParams.has("operation") ? [parsed.searchParams.get("operation")] : operations).map(operation=>({operation,eventCount:2}))};
  });
  await view.load();
  assert.equal(byId("access-summary").children.length, 6);
  assert.equal(byId("access-summary").children[5].tag, "details");
  assert.equal(byId("access-operation").children.find(node=>node.value==="consolidate_pages").textContent, "Consolidate memories");
  byId("access-operation").value = "read_pages"; byId("access-operation").handlers.change();
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(calls[1].searchParams.get("operation"), "read_pages");
  assert.equal(byId("access-operation").children.length, 8);
  assert.equal(byId("access-reset").hidden, false);
  byId("access-reset").handlers.click();
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(calls[2].searchParams.has("operation"), false);
  assert.equal(calls[2].searchParams.has("principalId"), false);
  assert.equal(calls[2].searchParams.get("view"), "requests");
  assert.equal(byId("access-reset").hidden, true);
});

test("audit view uses full server counts and server cursor; client selection resets paging", async () => {
  const calls = [];
  const { view, byId } = fixture(async url => { calls.push(new URL(url, "http://localhost")); return response(String(calls.length), "next-page"); });
  await view.load();
  assert.equal(calls[0].searchParams.get("includeHealthChecks"), "false");
  assert.equal(byId("access-loaded").textContent, "1 / 1234 requests");
  await view.load({ append: true });
  assert.equal(calls[1].searchParams.get("cursor"), "next-page");
  assert.equal(byId("access-loaded").textContent, "2 / 1234 requests");
  byId("access-clients").children[1].onclick();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls[2].searchParams.get("principalId"), "client:a");
  assert.equal(calls[2].searchParams.has("cursor"), false);
  assert.equal(byId("access-loaded").textContent, "1 / 1234 requests");
  assert.equal(byId("access-selected").textContent, "Client A");
});

test("a stale audit response cannot overwrite a newer filter or clear its loading state", async () => {
  const pending = [];
  const { view, byId } = fixture(() => new Promise(resolve => pending.push(resolve)));
  const old = view.load();
  byId("access-operation").value = "search_pages";
  const latest = view.load();
  pending[0](response("old")); await old;
  assert.equal(byId("access-rows").attributes["aria-busy"], "true");
  assert.equal(view.loaded, false);
  pending[1]({ ...response("new"), totalEvents: 7 }); await latest;
  assert.equal(byId("access-loaded").textContent, "1 / 7 requests");
  assert.equal(byId("access-rows").attributes["aria-busy"], "false");
});


test("request children are fetched only on expansion and use independent pagination", async () => {
  const calls = [];
  const { view, byId } = fixture(async url => {
    calls.push(new URL(url, "http://localhost"));
    if (calls.length > 1) {
      const result = response("child"); result.events[0].telemetry = {durationMs:2,projections:["payload","summary"]};
      return { ...result, nextCursor: calls.length === 2 ? "children-next" : null };
    }
    const result = response("root"); result.events[0].telemetry = {durationMs: 3, request: { id: "unique", root: true, internalOperations: 2, batchReads: 1, singleReads: 1, pageVisits: 9 }};
    return result;
  });
  await view.load();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].searchParams.get("view"), "requests");
  const details = byId("access-rows").children[0].children[4].children[0];
  const expanded = byId("access-rows").children[1];
  assert.equal(expanded.hidden, true);
  assert.equal(expanded.children[0].colSpan, 5);
  details.open = true; details.handlers.toggle();
  assert.equal(expanded.hidden, false);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls[1].searchParams.get("requestId"), "unique");
  assert.equal(calls[1].searchParams.get("view"), "operations");
  assert.match(text(expanded), /read_pages/);
  assert.match(text(expanded), /payload, summary/);
  details.handlers.toggle();
  assert.equal(calls.length, 2);
  expanded.children[0].children.at(-1).onclick();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls[2].searchParams.get("cursor"), "children-next");
});
