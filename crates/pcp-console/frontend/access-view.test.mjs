import test from "node:test";
import assert from "node:assert/strict";
import { createAccessView } from "../src/access-view.js";

class Node {
  constructor(text = "") { this.textContent = text; this.children = []; this.value = ""; this.checked = false; this.handlers = {}; this.attributes = {}; }
  append(...nodes) { this.children.push(...nodes); }
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

test("long audit scope lists start collapsed with two names and keep every name and identifier in disclosure", async () => {
  const scopes = ["project:one", "project:retired", "project:three", "project:missing"];
  const names = {"project:one":"Project One", "project:retired":"Historical project", "project:three":"Project Three"};
  const { view, byId } = fixture(async () => {
    const result = response("long");
    result.events[0].scopes = scopes; result.scopeNames = names;
    return result;
  });
  await view.load();
  const details = byId("access-rows").children[0].children[3].children[0];
  assert.equal(details.tag, "details");
  assert.ok(!details.open);
  assert.equal(text(details.children[0]), "Project One, Historical project4 scopes in total");
  assert.doesNotMatch(text(details.children[0]), /project:|Project Three/);
  const items = details.children[1].children;
  assert.deepEqual(items.map(item => item.children[0].textContent), ["Project One", "Historical project", "Project Three", "Unknown scope"]);
  assert.deepEqual(items.map(item => item.children[1].textContent), scopes);
});

test("short lists stay direct and appended audit pages retain earlier historical names", async () => {
  let calls = 0;
  const { view, byId } = fixture(async () => {
    const result = response(String(++calls), calls === 1 ? "next-page" : null);
    result.events[0].scopes = calls === 1 ? ["project:retired"] : ["project:current", "project:new"];
    result.scopeNames = calls === 1 ? {"project:retired":"Historical project"} : {"project:current":"Old current name", "project:new":"New project"};
    return result;
  }, {scopeName: scope => scope === "project:current" ? "Current project" : scope});
  await view.load(); await view.load({append:true});
  const rows = byId("access-rows").children;
  assert.equal(rows[0].children[3].textContent, "Historical project");
  assert.equal(rows[2].children[3].textContent, "Current project, New project");
  assert.equal(rows[2].children[3].children.length, 0);
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
    if (calls.length > 1) return { ...response("child"), nextCursor: calls.length === 2 ? "children-next" : null };
    const result = response("root"); result.events[0].telemetry = {durationMs: 3, request: { id: "unique", root: true, internalOperations: 2, batchReads: 1, singleReads: 1, pageVisits: 9 }};
    return result;
  });
  await view.load();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].searchParams.get("view"), "requests");
  const details = byId("access-rows").children[0].children[5].children[0];
  const expanded = byId("access-rows").children[1];
  assert.equal(expanded.hidden, true);
  assert.equal(expanded.children[0].colSpan, 6);
  details.open = true; details.handlers.toggle();
  assert.equal(expanded.hidden, false);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls[1].searchParams.get("requestId"), "unique");
  assert.equal(calls[1].searchParams.get("view"), "operations");
  details.handlers.toggle();
  assert.equal(calls.length, 2);
  expanded.children[0].children.at(-1).onclick();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls[2].searchParams.get("cursor"), "children-next");
});
