import test from "node:test";
import assert from "node:assert/strict";
import { createLocalClientSetup } from "../src/local-client-setup.js";

class Node {
  constructor(tag = "div", text = "") { this.tagName = tag.toUpperCase(); this.textContent = text; this.children = []; this.dataset = {}; this.handlers = {}; this.attributes = {}; this.value = ""; this.open = false; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(key, value) { this.attributes[key] = value; }
  addEventListener(key, value) { this.handlers[key] = value; }
  querySelectorAll(selector) { return this.children.flatMap(child => [...(selector === "input" && child.tagName === "INPUT" ? [child] : []), ...child.querySelectorAll(selector)]); }
  focus() { document.activeElement = this; }
  showModal() { this.open = true; }
  close() { this.open = false; }
}

const id = "cb9876ba-a25f-4478-a482-af3af7400872";
const profile = (overrides = {}) => ({ id, displayName: "Research helper", principalId: "client:local-helper", mode: "read", scopes: ["project:one"], status: "pending", requestId: "req_local", ...overrides });
const scopes = [{ namespace: "user:self", displayName: "My memory" }, { namespace: "project:one", displayName: "Project One" }, { namespace: "project:two", displayName: "Project Two" }];
const configuration = { mcpServers: { [`pcp-${id}`]: { command: "/opt/pcp/bin/pcp-mcp", args: [], env: { PCP_CLIENT_ID: "client:local-helper", PCP_ENROLLMENT_FILE: `/Users/test/.pcp/local-clients/${id}/credentials.json`, PCP_MCP_SURFACE: "generic", PCP_MCP_TOOLSET: "context" } } } };
const text = node => [node.textContent, ...node.children.map(text)].join(" ");

function fixture({ request, write, confirmation = false } = {}) {
  let current = profile(), language = "en";
  const nodes = new Map(), reads = [], writes = [], copied = [], downloaded = [], confirmations = [];
  const byId = name => { if (!nodes.has(name)) nodes.set(name, new Node()); return nodes.get(name); };
  globalThis.document = { activeElement: null };
  const view = createLocalClientSetup({ byId, element: (tag, cls, value) => Object.assign(new Node(tag, value), { className: cls }),
    t: value => language === "zh" ? `中:${value}` : value, scopeName: namespace => namespace,
    operationId: () => id, copy: async value => copied.push(value), download: (...args) => downloaded.push(args),
    confirmAction: async value => { confirmations.push(value); return typeof confirmation === "function" ? confirmation() : confirmation; },
    api: async path => {
      reads.push(path); if (request) return request(path, current);
      if (path === "/api/local-clients") return { available: true, clients: [current], scopeOptions: scopes };
      if (path.endsWith("/config")) return { client: current, config: configuration };
      return current;
    },
    mutate: async (path, body) => {
      writes.push([path, body]); if (write) return write(path, body, current);
      current = path.endsWith("/approve") ? { ...current, status: "ready", registrationId: "reg_local" } : profile({ ...body, id: body.operationId });
      return current;
    },
  });
  const name = async value => { byId("local-client-name").value = value; await byId("local-client-name").handlers.input(); };
  const choose = async (namespace, checked = true) => {
    const input = byId("local-client-scope-options").querySelectorAll("input").find(node => node.value === namespace);
    assert.ok(input); input.checked = checked; await input.handlers.change();
  };
  const register = () => byId("local-client-form").handlers.submit({ preventDefault() {} });
  return { view, byId, reads, writes, copied, downloaded, confirmations, name, choose, register,
    setCurrent: value => { current = value; }, language: value => { language = value; } };
}

test("a new local agent defaults to read and requires an explicit scope before registration", async () => {
  const f = fixture(); await f.view.load(); await f.view.openNew();
  assert.equal(f.byId("local-client-mode").value, "read");
  assert.ok(f.byId("local-client-scope-options").querySelectorAll("input").every(input => !input.checked));
  await f.name("Research helper"); await f.register();
  assert.equal(f.writes.length, 0);
  assert.match(f.byId("local-client-error-message").textContent, /choose at least one/);
  await f.choose("project:one"); await f.register();
  assert.deepEqual(f.writes, [["/api/local-clients", { operationId: id, displayName: "Research helper", mode: "read", scopes: ["project:one"] }]]);
  assert.equal(f.byId("local-client-form").hidden, true);
  assert.equal(f.byId("local-client-approve").hidden, false);
  assert.match(text(f.byId("local-client-profile-scopes")), /Project One/);
  assert.doesNotMatch(text(f.byId("local-client-profile-scopes")), /Project Two|Personal memory/);
  assert.equal(f.byId("local-client-config-section").hidden, true);
});

test("only explicit authorization exposes the exact complete MCP configuration and it can be reopened", async () => {
  const f = fixture(); await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  assert.equal(f.writes.length, 1);
  await f.byId("local-client-approve").handlers.click();
  assert.deepEqual(f.writes[1], [`/api/local-clients/${id}/approve`, undefined]);
  assert.equal(f.byId("local-client-profile-status").textContent, "Ready to connect");
  assert.deepEqual(JSON.parse(f.byId("local-client-config").value), configuration);
  assert.equal(f.byId("local-client-copy").disabled, false);
  await f.byId("local-client-copy").handlers.click(); await f.byId("local-client-download").handlers.click();
  assert.deepEqual(JSON.parse(f.copied[0]), configuration);
  assert.equal(f.downloaded[0][1], `pcp-${id}.json`);
  assert.equal(f.view.hasPendingAction(), false);
  await f.view.close(); await f.view.openProfile(id);
  assert.deepEqual(JSON.parse(f.byId("local-client-config").value), configuration);
  assert.equal(f.writes.length, 2);
});

test("scope search retains selections outside the filtered list and language changes preserve the draft", async () => {
  const f = fixture(); await f.view.load(); await f.view.openNew(); await f.name("研究助手"); await f.choose("user:self");
  f.byId("local-client-scope-search").value = "Project Two"; await f.byId("local-client-scope-search").handlers.input();
  assert.equal(f.byId("local-client-scope-options").querySelectorAll("input").length, 1);
  await f.choose("project:two"); f.language("zh"); f.view.rerender();
  assert.equal(f.byId("local-client-name").value, "研究助手");
  f.byId("local-client-scope-search").value = ""; await f.byId("local-client-scope-search").handlers.input();
  assert.match(text(f.byId("local-client-scope-options")), /中:Personal memory/);
  assert.deepEqual(f.byId("local-client-scope-options").querySelectorAll("input").filter(input => input.checked).map(input => input.value), ["user:self", "project:two"]);
  await f.register(); assert.deepEqual(f.writes[0][1].scopes, ["user:self", "project:two"]);
});

test("an unknown registration is never replayed and read-only recovery still requires explicit authorization", async () => {
  let fail = true;
  const f = fixture({ write: async (path, body) => { if (fail) throw new Error("response lost"); return profile({ status: "ready" }); } });
  await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  assert.equal(f.view.blocksMutation(), true); assert.equal(f.view.hasPendingAction(), true);
  assert.match(f.byId("local-client-error-message").textContent, /outcome is unknown.*\nresponse lost/);
  assert.equal(document.activeElement, f.byId("local-client-error"));
  await f.register(); await f.view.load(); assert.equal(f.writes.length, 1);
  await f.byId("local-client-check").handlers.click();
  assert.ok(f.reads.includes(`/api/local-clients/${id}`));
  assert.equal(f.view.blocksMutation(), false); assert.equal(f.byId("local-client-approve").disabled, false);
  assert.equal(f.writes.length, 1);
  fail = false; await f.byId("local-client-approve").handlers.click();
  assert.equal(f.writes.length, 2); assert.match(f.writes[1][0], /\/approve$/);
});

test("an unknown approval recovers an actual ready grant without sending a second approval", async () => {
  const f = fixture({ write: async path => { if (path.endsWith("/approve")) { f.setCurrent(profile({ status: "ready" })); throw new Error("response lost"); } return profile(); } });
  await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  await f.byId("local-client-approve").handlers.click();
  assert.equal(f.view.blocksMutation(), true); assert.equal(f.byId("local-client-copy").disabled, true);
  await f.byId("local-client-approve").handlers.click(); assert.equal(f.writes.length, 2);
  await f.byId("local-client-check").handlers.click();
  assert.equal(f.writes.length, 2); assert.equal(f.view.blocksMutation(), false);
  assert.deepEqual(JSON.parse(f.byId("local-client-config").value), configuration);
});

test("interrupted registration resumes only after a fresh structured check and an explicit click", async () => {
  let first = true;
  const f = fixture({ write: async (path, body) => {
    if (first) {
      first = false; f.setCurrent(profile({ status: "unavailable", canResumeRegistration: true, reason: "Registration was interrupted" }));
      throw new Error("response lost");
    }
    f.setCurrent(profile()); return profile();
  } });
  await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  await f.view.load();
  assert.equal(f.byId("local-client-resume").hidden, true);
  await f.byId("local-client-resume").handlers.click(); assert.equal(f.writes.length, 1);
  await f.byId("local-client-check").handlers.click();
  assert.equal(f.byId("local-client-resume").hidden, false);
  assert.equal(f.byId("local-client-resume").disabled, false); assert.equal(f.view.blocksMutation(), true);
  assert.equal(f.writes.length, 1);
  await f.byId("local-client-resume").handlers.click();
  assert.deepEqual(f.writes[1], f.writes[0]);
  assert.equal(f.writes.length, 2); assert.equal(f.byId("local-client-resume").hidden, true);
  assert.equal(f.byId("local-client-approve").hidden, false); assert.equal(f.view.blocksMutation(), false);
  assert.ok(f.writes.every(([path]) => !path.endsWith("/approve")));
});

test("a reloaded interrupted profile can resume with its saved descriptor while a reason alone cannot enable retry", async () => {
  const f = fixture(); f.setCurrent(profile({ status: "unavailable", canResumeRegistration: true }));
  await f.view.load(); assert.equal(f.byId("local-client-resume").hidden, true);
  await f.view.openProfile(id); assert.equal(f.byId("local-client-resume").hidden, false);
  await f.byId("local-client-resume").handlers.click();
  assert.deepEqual(f.writes[0], ["/api/local-clients", { operationId: id, displayName: "Research helper", mode: "read", scopes: ["project:one"] }]);
  const unavailable = fixture(); unavailable.setCurrent(profile({ status: "unavailable", reason: "Retry the registration" }));
  await unavailable.view.load(); await unavailable.view.openProfile(id);
  assert.equal(unavailable.byId("local-client-resume").hidden, true); assert.equal(unavailable.writes.length, 0);
});

test("authoritative absence after a lost registration response preserves an editable draft without replay", async () => {
  const f = fixture({ write: async () => { throw new Error("response lost"); }, request: async path => {
    if (path === "/api/local-clients") return { available: true, clients: [], scopeOptions: scopes };
    throw Object.assign(new Error("not found"), { status: 404 });
  } });
  await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  await f.byId("local-client-check").handlers.click();
  assert.equal(f.view.blocksMutation(), false); assert.equal(f.byId("local-client-form").hidden, false);
  assert.equal(f.byId("local-client-name").value, "Research helper");
  assert.match(f.byId("local-client-notice").textContent, /No registration was found/); assert.equal(f.writes.length, 1);
});

test("unavailable recovery keeps an unknown setup frozen and rejects a different saved grant", async () => {
  let unavailable = true;
  const f = fixture({ write: async () => { throw new Error("response lost"); }, request: async path => {
    if (path === "/api/local-clients") return { available: true, clients: [], scopeOptions: scopes };
    if (unavailable) throw new Error("Runtime unavailable");
    return profile({ scopes: ["user:self"] });
  } });
  await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  await f.byId("local-client-check").handlers.click();
  assert.equal(f.view.blocksMutation(), true); assert.match(f.byId("local-client-error-message").textContent, /Runtime unavailable/);
  unavailable = false; await f.byId("local-client-check").handlers.click();
  assert.equal(f.view.blocksMutation(), true); assert.match(f.byId("local-client-error-message").textContent, /permissions differ/); assert.equal(f.writes.length, 1);
});

test("a known rejected registration stays editable and Unicode names obey the backend byte bound", async () => {
  const f = fixture({ write: async () => { throw Object.assign(new Error("scope rejected"), { mutationOutcome: "not_applied" }); } });
  await f.view.load(); await f.view.openNew(); await f.name("研".repeat(41)); await f.choose("project:one"); await f.register();
  assert.equal(f.writes.length, 0); assert.match(f.byId("local-client-error-message").textContent, /name is too long/);
  await f.name("研究助手"); await f.register();
  assert.equal(f.view.blocksMutation(), false); assert.equal(f.byId("local-client-name").disabled, false);
  assert.equal(f.byId("local-client-name").value, "研究助手"); assert.doesNotMatch(f.byId("local-client-error-message").textContent, /unknown/);
});

test("close and Escape protect an unregistered draft and confirmed discard clears its protection", async () => {
  let confirm = false;
  const f = fixture({ confirmation: () => confirm }); await f.view.load(); await f.view.openNew(); await f.name("Unsaved helper");
  assert.equal(await f.view.close(), false); assert.equal(f.byId("local-client-dialog").open, true);
  let prevented = false; await f.byId("local-client-dialog").handlers.cancel({ preventDefault() { prevented = true; } });
  assert.equal(prevented, true); assert.equal(f.view.hasPendingAction(), true);
  confirm = true; assert.equal(await f.view.close(), true); assert.equal(f.view.hasPendingAction(), false);
  await f.view.openNew(); assert.equal(f.byId("local-client-name").value, "");
});

test("closing an unknown setup retains recovery and an unavailable inventory does not erase prior profiles", async () => {
  let offline = false;
  const f = fixture({ confirmation: true, write: async () => { throw new Error("response lost"); }, request: async path => {
    if (offline) throw new Error("offline");
    return { available: true, clients: [profile()], scopeOptions: scopes };
  } });
  await f.view.load(); await f.view.openNew(); await f.name("Research helper"); await f.choose("project:one"); await f.register();
  await f.view.close(); assert.equal(f.view.hasPendingAction(), true);
  await f.view.openNew(); assert.equal(f.byId("local-client-form").hidden, true); assert.equal(f.writes.length, 1);
  offline = true; await f.view.load();
  assert.equal(f.view.snapshot().available, false); assert.equal(f.view.snapshot().clients.length, 1);
  assert.equal(f.view.blocksMutation(), true); assert.equal(f.writes.length, 1);
});

test("a failed current-state read cannot leave stale approval or configuration controls enabled", async () => {
  const f = fixture({ request: async path => {
    if (path === "/api/local-clients") return { available: true, clients: [profile()], scopeOptions: scopes };
    throw new Error("offline");
  } });
  await f.view.load(); await f.view.openProfile(id);
  assert.equal(f.byId("local-client-profile-status").textContent, "State unavailable");
  assert.equal(f.byId("local-client-approve").hidden, true); assert.equal(f.byId("local-client-copy").disabled, true);
  assert.equal(f.view.blocksMutation(), false); assert.equal(f.writes.length, 0);
});
