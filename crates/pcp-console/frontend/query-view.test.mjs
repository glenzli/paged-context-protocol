import assert from "node:assert/strict";
import test from "node:test";

import { buildQueryRequest, contextValidityLabel, createQueryView, querySubmitLabel } from "../src/query_view.js";

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

test("query submit keeps its result meaning while exposing explicit busy labels", () => {
  assert.equal(querySubmitLabel("semantic_search", false), "Build context pack");
  assert.equal(querySubmitLabel("semantic_search", true), "Searching context…");
  assert.equal(querySubmitLabel("match_intent", true), "Matching intent…");
});

test("context validity standings use explicit review labels", () => {
  assert.equal(contextValidityLabel("qualified"), "Qualified");
  assert.equal(contextValidityLabel("disputed"), "Disputed");
  assert.equal(contextValidityLabel("custom"), "custom");
});

// Exercise the real view, event handlers and async responses with a small DOM adapter.
class ViewNode {
  constructor(tag = "div", value = "") {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.parentElement = null;
    this.attributes = {};
    this.dataset = {};
    this.listeners = {};
    this.value = "";
    this.hidden = false;
    this._text = String(value ?? "");
    this.classList = { toggle() {} };
  }
  set textContent(value) { this.replaceChildren(); this._text = String(value); }
  get textContent() { return this._text + this.children.map(node => node.textContent).join(""); }
  append(...nodes) { nodes.forEach(node => { node.remove(); node.parentElement = this; this.children.push(node); }); }
  prepend(...nodes) { nodes.forEach(node => node.remove()); nodes.forEach(node => { node.parentElement = this; }); this.children.unshift(...nodes); }
  replaceChildren(...nodes) { this.children.forEach(node => { node.parentElement = null; }); this.children = []; this._text = ""; this.append(...nodes); }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(node => node !== this); this.parentElement = null; }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  addEventListener(key, handler) { (this.listeners[key] ||= []).push(handler); }
  async fire(key) { for (const handler of this.listeners[key] || []) await handler({ target: this, preventDefault() {} }); }
  requestSubmit() { return this.fire("submit"); }
  all() { return [this, ...this.children.flatMap(node => node.all())]; }
}

const pack = (content = "Completed context", scope = "project:one") => ({
  visibility: "explicit", scopes: [scope], anchorCount: 1, contextBudgetChars: 4000,
  entries: [{ rank: 1, anchorRank: 1, detail: "payload", namespace: scope, pageId: "page-one", revisionId: "rev-one", kind: "document", content, matchedBy: "payload" }],
});
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const settle = () => new Promise(resolve => setImmediate(resolve));

function queryFixture(context, { post = async () => pack(), audit = async () => ({ calls: 2 }) } = {}) {
  const root = new ViewNode("main"), calls = [], errors = [], intervals = new Map();
  const element = (tag, className, value) => Object.assign(new ViewNode(tag, value), { className });
  const add = (parent, id, tag = "div", value = "") => {
    const node = element(tag, ""); node.id = id; node.value = value; parent.append(node); return node;
  };
  const form = add(root, "context-query-form", "form");
  add(form, "context-query-text", "textarea", "PCP evidence");
  add(form, "context-query-method", "select", "semantic_search");
  add(add(form, "context-query-effort-control"), "context-query-effort", "select", "medium");
  add(form, "context-query-scope", "select", "");
  add(form, "context-query-top-k", "select", "6");
  add(form, "context-query-submit", "button");
  add(form, "context-query-method-note", "p");
  const error = add(form, "context-query-error");
  add(add(error, "error-copy"), "context-query-error-message", "span");
  add(error, "context-query-retry", "button");
  add(root, "context-pack-results");
  const preview = add(root, "context-model-preview", "details");
  add(preview, "context-model-context", "pre");
  add(preview, "context-model-context-status", "span");
  add(root, "context-query-audit", "section");
  add(root, "query-status", "span");
  const byId = id => root.all().find(node => node.id === id) || null;
  const previous = { document: globalThis.document, window: globalThis.window, Option: globalThis.Option };
  globalThis.document = { createTextNode: value => new ViewNode("#text", value) };
  globalThis.window = { setInterval: fn => { const id = intervals.size + 1; intervals.set(id, fn); return id; }, clearInterval: id => intervals.delete(id) };
  globalThis.Option = class extends ViewNode { constructor(text, value) { super("option", text); this.value = value; } };
  context.after(() => Object.assign(globalThis, previous));
  let now = Date.parse("2026-10-22T10:00:00.000Z"), locale = "en";
  context.mock.method(Date, "now", () => now);
  const translations = {
    "Executed query": "已执行的查询", "Executed at": "执行时间", "Query failed": "查询失败",
    "Semantic search": "语义搜索", "Match intent": "意图匹配", "Retrieval effort": "检索投入", High: "高",
    "Conditions changed; run the query again to update results.": "条件已修改，尚未重新查询。",
    "The latest query failed; the previous completed results remain below.": "最近一次查询失败；下方保留上次完成的结果。",
  };
  const view = createQueryView({
    request: (path, options) => {
      if (!options) return audit(path);
      const call = { path, payload: JSON.parse(options.body) }; calls.push(call); return post(call);
    },
    byId, element, showError: error => errors.push(error),
    t: value => locale === "zh" ? translations[value] || value : value,
    formatNumber: String, formatTime: value => `${locale}:${value}`,
    scopeName: value => ({ "project:one": "Mini Wiki", "project:two": "Math Workspace" }[value] || value),
    openPage: async () => {}, openPageIcon: () => element("svg"), searchIcon: () => element("svg"),
  });
  view.setScopes([{ namespace: "project:one", displayName: "Mini Wiki" }, { namespace: "project:two", displayName: "Math Workspace" }]);
  return {
    view, byId, calls, errors,
    submit: () => form.fire("submit"),
    async change(id, value, event = "change") { byId(id).value = value; await byId(id).fire(event); },
    advance(ms) { now += ms; },
    tick() { intervals.forEach(fn => fn()); },
    set locale(value) { locale = value; },
  };
}

test("completed results keep their executed conditions while form edits only mark them as outdated", async context => {
  const f = queryFixture(context);
  await f.view.load();
  await f.change("context-query-scope", "project:one");
  await f.change("context-query-text", "  PCP evidence  ", "input");
  await f.submit();
  assert.deepEqual(f.calls, [{ path: "/api/query/semantic-search", payload: { query: "PCP evidence", scopes: ["project:one"], resultLimit: 6 } }]);
  const snapshot = f.byId("context-query-snapshot"), original = snapshot.textContent;
  const article = f.byId("context-pack-results").children.find(node => node.tagName === "ARTICLE");
  assert.match(original, /Executed queryPCP evidenceScopeMini WikiRetrieval methodSemantic searchTop results6Executed aten:2026-10-22T10:00:00.000Z/);
  assert.doesNotMatch(original, /Retrieval effort/);
  assert.equal(snapshot.all().find(node => node.attributes["data-scope-names"]).attributes["data-scope-names"], '["project:one"]');
  assert.equal(snapshot.all().find(node => node.tagName === "TIME").attributes.datetime, "2026-10-22T10:00:00.000Z");
  for (const [id, value, restored, event] of [
    ["context-query-text", "New evidence", "PCP evidence", "input"],
    ["context-query-method", "match_intent", "semantic_search"],
    ["context-query-scope", "project:two", "project:one"],
    ["context-query-top-k", "12", "6"],
  ]) {
    await f.change(id, value, event);
    assert.match(f.byId("context-query-condition-status").textContent, /Conditions changed/);
    assert.equal(f.byId("context-query-snapshot"), snapshot);
    assert.equal(snapshot.textContent, original);
    assert.equal(f.byId("context-pack-results").children.find(node => node.tagName === "ARTICLE"), article);
    await f.change(id, restored, event);
    assert.equal(f.byId("context-query-condition-status").textContent, "These results match the current conditions.");
  }
  assert.equal(f.calls.length, 1);
  assert.match(f.byId("query-status").textContent, /Mini Wiki/);
  assert.equal(f.byId("context-query-condition-status").attributes["aria-live"], "polite");
  await f.change("context-query-effort", "high");
  assert.equal(f.byId("context-query-condition-status").textContent, "These results match the current conditions.");
  assert.deepEqual(f.errors, []);
});

test("an empty successful intent query still records its scope, limit, effort and execution time", async context => {
  const f = queryFixture(context, { post: async () => ({ entries: [], scopes: [], anchorCount: 0 }) });
  await f.change("context-query-method", "match_intent");
  await f.change("context-query-text", "第一行\n第二行", "input");
  await f.change("context-query-effort", "high");
  await f.change("context-query-top-k", "12");
  await f.submit();
  assert.deepEqual(f.calls[0].payload, { query: "第一行\n第二行", scopes: [], resultLimit: 12, intentEffort: "high" });
  assert.match(f.byId("context-query-snapshot").textContent, /第一行\n第二行ScopeAll authorized scopesRetrieval methodMatch intentTop results12Retrieval effortHighExecuted at/);
  assert.match(f.byId("context-pack-results").textContent, /No results/);
  await f.change("context-query-effort", "medium");
  assert.match(f.byId("context-query-condition-status").textContent, /Conditions changed/);
});

test("late completion and a locale rerender preserve the sent request instead of relabeling it with new inputs", async context => {
  const response = deferred(), f = queryFixture(context, { post: () => response.promise });
  await f.change("context-query-scope", "project:one");
  const running = f.submit();
  await f.change("context-query-text", "Next question", "input");
  await f.change("context-query-method", "match_intent");
  await f.change("context-query-top-k", "20");
  f.view.setScopes([{ namespace: "project:two", displayName: "Math Workspace" }]);
  f.locale = "zh";
  f.view.rerender();
  f.advance(2500); f.tick();
  assert.equal(f.byId("context-query-scope").value, "");
  assert.match(f.byId("query-status").textContent, /Semantic search in progress · 2 seconds/);
  assert.equal(f.byId("context-query-submit").title, "Searching context…");
  assert.match(f.byId("context-query-condition-status").textContent, /条件已修改/);
  response.resolve(pack("Original question result"));
  await running;
  assert.match(f.byId("context-query-snapshot").textContent, /已执行的查询PCP evidenceScopeMini WikiRetrieval method语义搜索Top results6执行时间zh:2026-10-22T10:00:00.000Z/);
  assert.doesNotMatch(f.byId("context-query-snapshot").textContent, /Next question|意图匹配|Top results20/);
  assert.equal(f.byId("context-query-text").value, "Next question");
  assert.equal(f.byId("context-query-method").value, "match_intent");
  assert.match(f.byId("context-query-condition-status").textContent, /条件已修改/);
  assert.equal(f.byId("context-query-submit").disabled, false);
  assert.equal(f.byId("context-pack-results").attributes["aria-busy"], "false");
});

test("a failed retry retains the completed snapshot and binds its error to the failed attempt across locale and input changes", async context => {
  let attempt = 0;
  const failure = deferred();
  const f = queryFixture(context, { post: () => ++attempt === 2 ? failure.promise : Promise.resolve(pack(`Result ${attempt}`)) });
  await f.submit();
  const completed = f.byId("context-query-snapshot").textContent;
  f.advance(60000);
  await f.change("context-query-text", "Failed question", "input");
  await f.change("context-query-method", "match_intent");
  await f.change("context-query-effort", "high");
  await f.change("context-query-scope", "project:two");
  await f.change("context-query-top-k", "12");
  const retry = f.submit();
  assert.equal(f.byId("context-query-snapshot").textContent, completed);
  assert.match(f.byId("context-query-condition-status").textContent, /current query is running.*previous completed results/);
  failure.reject(new Error("Router unavailable"));
  await retry;
  assert.equal(f.byId("context-query-snapshot").textContent, completed);
  assert.match(f.byId("context-query-error-attempt").textContent, /Failed question.*Scope: Math Workspace.*Retrieval method: Match intent.*Top results: 12.*Retrieval effort: High.*2026-10-22T10:01:00.000Z/);
  assert.match(f.byId("context-query-condition-status").textContent, /latest query failed.*previous completed results/);
  assert.equal(f.byId("query-status").textContent, "Query failed");
  f.locale = "zh"; f.view.rerender();
  assert.equal(f.byId("query-status").textContent, "查询失败");
  assert.match(f.byId("context-query-condition-status").textContent, /最近一次查询失败/);
  assert.match(f.byId("context-query-error-attempt").textContent, /意图匹配.*检索投入: 高.*zh:2026-10-22T10:01:00.000Z/);
  await f.change("context-query-text", "Reviewed question", "input");
  assert.match(f.byId("context-query-error-attempt").textContent, /Failed question/);
  assert.doesNotMatch(f.byId("context-query-error-attempt").textContent, /Reviewed question/);
  f.advance(60000);
  await f.byId("context-query-retry").fire("click");
  await settle();
  assert.equal(f.calls.length, 3);
  assert.equal(f.calls[2].payload.query, "Reviewed question");
  assert.match(f.byId("context-query-snapshot").textContent, /Reviewed question.*2026-10-22T10:02:00.000Z/);
  assert.equal(f.byId("context-query-error").hidden, true);
  assert.equal(f.byId("context-query-error-attempt"), null);
  assert.doesNotMatch(f.byId("query-status").textContent, /查询失败/);
  assert.deepEqual(f.errors, []);
});

test("scope diagnostics ignore older responses and do not keep a completed query busy", async context => {
  const audits = [];
  const f = queryFixture(context, { audit: path => { const response = deferred(); audits.push({ path, ...response }); return response.promise; } });
  const loading = f.view.load();
  audits[0].resolve({ calls: 1 }); await loading;
  await f.change("context-query-scope", "project:one");
  await f.change("context-query-scope", "project:two");
  assert.match(audits[1].path, /scope=project%3Aone/);
  assert.match(audits[2].path, /scope=project%3Atwo/);
  assert.equal(f.byId("context-query-audit").hidden, true);
  audits[2].resolve({ calls: 9 }); await settle();
  const latest = f.byId("context-query-audit").textContent;
  assert.match(latest, /Scope: Math Workspace.*Calls9/);
  audits[1].resolve({ calls: 99 }); await settle();
  assert.equal(f.byId("context-query-audit").textContent, latest);
  await f.submit();
  assert.equal(f.byId("context-query-submit").disabled, false);
  assert.equal(f.byId("context-query-condition-status").textContent, "These results match the current conditions.");
  assert.equal(f.byId("context-query-audit").hidden, true);
  audits.at(-1).reject(new Error("Audit permission unavailable")); await settle();
  assert.equal(f.byId("context-query-error").hidden, true);
  assert.deepEqual(f.errors, []);
});

test("failed conditions are distinguished from new edits and reverting to completed conditions", async context => {
  let attempt = 0;
  const f = queryFixture(context, { post: async () => {
    if (++attempt === 2) throw new Error("Search unavailable");
    return pack();
  } });
  await f.submit();
  const completed = f.byId("context-query-snapshot");
  await f.change("context-query-text", "Failed query", "input");
  await f.submit();
  const failureMessage = "The latest query failed; the previous completed results remain below.";
  assert.equal(f.byId("context-query-condition-status").textContent, failureMessage);
  await f.change("context-query-text", "New query", "input");
  assert.match(f.byId("context-query-condition-status").textContent, /Conditions changed/);
  await f.change("context-query-text", "PCP evidence", "input");
  assert.equal(f.byId("context-query-condition-status").textContent, failureMessage);
  await f.change("context-query-text", "  Failed query  ", "input");
  assert.equal(f.byId("context-query-condition-status").textContent, failureMessage);
  assert.equal(f.byId("context-query-snapshot"), completed);
  assert.equal(f.calls.length, 2);
});
