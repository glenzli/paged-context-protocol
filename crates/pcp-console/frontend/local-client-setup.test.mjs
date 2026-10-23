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

function fixture({ request, write, confirmation = false, preferences = new Map() } = {}) {
  let current = profile(), language = "en";
  const nodes = new Map(), reads = [], writes = [], copied = [], downloaded = [], confirmations = [];
  const byId = name => { if (!nodes.has(name)) nodes.set(name, new Node()); return nodes.get(name); };
  globalThis.document = { activeElement: null };
  const view = createLocalClientSetup({ byId, element: (tag, cls, value) => Object.assign(new Node(tag, value), { className: cls }),
    t: value => language === "zh" ? `中:${value}` : value, scopeName: namespace => namespace,
    selection: { get: id => preferences.get(id), set: (id, value) => preferences.set(id, value) },
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
  const scopeMode = async value => { byId("local-client-scope-mode").value = value; await byId("local-client-scope-mode").handlers.change(); };
  const choose = async (namespace, checked = true) => {
    if (byId("local-client-scope-mode").value !== "selected") await scopeMode("selected");
    const input = byId("local-client-scope-options").querySelectorAll("input").find(node => node.value === namespace);
    assert.ok(input); input.checked = checked; await input.handlers.change();
  };
  const register = () => byId("local-client-form").handlers.submit({ preventDefault() {} });
  return { view, byId, reads, writes, copied, downloaded, confirmations, name, choose, scopeMode, register,
    setCurrent: value => { current = value; }, language: value => { language = value; } };
}

test("new registrations default to persistent all scopes without an implicit approval", async () => {
  const f = fixture(); await f.view.load(); await f.view.openNew();
  assert.equal(f.byId("local-client-target").children[0].textContent, "Generic Agent");
  assert.equal(f.byId("local-client-mode").value, "read");
  assert.equal(f.byId("local-client-scope-mode").value, "all");
  assert.equal(f.byId("local-client-selected-scopes").hidden, true);
  assert.equal(f.view.hasPendingAction(), false);
  await f.name("All scope reader"); await f.register();
  assert.deepEqual(f.writes, [["/api/local-clients", { operationId: id, displayName: "All scope reader", mode: "read", allScopes: true, scopes: [] }]]);
  assert.equal(text(f.byId("local-client-profile-scopes")).trim(), "All scopes");
  assert.equal(f.byId("local-client-approve").hidden, false);
  assert.equal(f.byId("local-client-config-section").hidden, true);
});

test("switching all/selected retains the checklist but only submits the active scope policy", async () => {
  const f = fixture(); await f.view.load(); await f.view.openNew(); await f.name("Contributor");
  await f.choose("project:one"); await f.scopeMode("all");
  assert.equal(f.byId("local-client-selected-scopes").hidden, true);
  await f.scopeMode("selected");
  assert.ok(f.byId("local-client-scope-options").querySelectorAll("input").some(node => node.value === "project:one" && node.checked));
  await f.scopeMode("all");
  f.byId("local-client-mode").value = "contribute"; await f.byId("local-client-mode").handlers.change();
  await f.register();
  assert.equal(f.writes[0][1].allScopes, true); assert.deepEqual(f.writes[0][1].scopes, []);
  assert.equal(f.writes[0][1].mode, "contribute");
});

test("recovery refuses a changed scope policy even with the same identity and empty scopes", async () => {
  const f = fixture({write: async (path, body) => profile({...body, id, allScopes: false})});
  await f.view.load(); await f.view.openNew(); await f.name("All reader"); await f.register();
  assert.equal(f.view.blocksMutation(), true);
  assert.equal(f.byId("local-client-config-section").hidden, true);
});

test("selected-scope registration requires an explicit scope and shows only its grant", async () => {
  const f = fixture(); await f.view.load(); await f.view.openNew();
  assert.equal(f.byId("local-client-mode").value, "read");
  assert.ok(f.byId("local-client-scope-options").querySelectorAll("input").every(input => !input.checked));
  await f.scopeMode("selected"); await f.name("Research helper"); await f.register();
  assert.equal(f.writes.length, 0);
  assert.match(f.byId("local-client-error-message").textContent, /choose at least one/);
  await f.choose("project:one"); await f.register();
  assert.deepEqual(f.writes, [["/api/local-clients", { operationId: id, displayName: "Research helper", mode: "read", allScopes: false, scopes: ["project:one"] }]]);
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
  await f.register(); assert.deepEqual(f.writes[0][1].scopes, ["project:two", "user:self"]);
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
  assert.deepEqual(f.writes[0], ["/api/local-clients", { operationId: id, displayName: "Research helper", mode: "read", allScopes: false, scopes: ["project:one"] }]);
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


test("target selection survives approval, reload and locale changes without modifying grants", async () => {
  const preferences = new Map(), f = fixture({ preferences });
  await f.view.load(); await f.view.openNew();
  assert.equal(f.byId("local-client-target").children.length, 14);
  f.byId("local-client-target").value = "dsh"; await f.byId("local-client-target").handlers.change();
  await f.name("DSH helper"); await f.choose("project:one"); await f.register();
  await f.byId("local-client-approve").handlers.click();
  assert.match(f.byId("local-client-config").value, /^- insert:/);
  assert.match(f.byId("local-client-install-command").value, /^dsh web --patch /);
  assert.equal(f.byId("local-client-command-section").hidden, false);
  assert.equal(preferences.get(id), "dsh");
  f.language("zh"); f.view.rerender();
  assert.equal(f.byId("local-client-target").value, "dsh");
  assert.match(f.byId("local-client-verification").value, /^中:/);
  await f.byId("local-client-copy").handlers.click(); await f.byId("local-client-download").handlers.click();
  assert.equal(f.copied[0], f.downloaded[0][0]);
  assert.match(f.downloaded[0][1], /-dsh.cordis.yml$/);
  assert.equal(f.downloaded[0][2], "application/yaml");
  assert.equal(f.writes.length, 2);
  assert.equal(Object.hasOwn(f.writes[0][1], "target"), false);
  await f.view.close();
  const next = fixture({ preferences }); next.setCurrent(profile({ status: "ready" }));
  await next.view.load(); await next.view.openProfile(id);
  assert.equal(next.byId("local-client-target").value, "dsh");
  assert.equal(next.writes.length, 0);
});

test("switching formats changes all exports and verification namespace but retains the independent identity", async () => {
  const f = fixture(); f.setCurrent(profile({ status: "ready" }));
  await f.view.load(); await f.view.openProfile(id);
  for (const [target, starts] of [["claude-code", "{"], ["codex", "[mcp_servers."], ["dsh", "- insert:"], ["vscode", "{"]]) {
    f.byId("local-client-target").value = target; await f.byId("local-client-target").handlers.change();
    assert.ok(f.byId("local-client-config").value.startsWith(starts));
    assert.equal(f.byId("local-client-expected-identity").textContent, "client:local-helper");
    await f.byId("local-client-copy-verification").handlers.click();
    assert.ok(f.copied.at(-1).includes("client:local-helper"));
    assert.ok(f.copied.at(-1).includes("project:one"));
    if (target === "dsh") assert.match(f.copied.at(-1), /pcp_[a-z0-9]+/);
    assert.match(f.byId("local-client-guidance").value, /read-only/);
  }
  assert.deepEqual(f.writes, []);
  assert.equal(f.byId("local-client-command-section").hidden, true);
  assert.equal(f.byId("local-client-install-command").value, "");
  assert.equal(f.byId("local-client-profile-status").textContent, "Ready to connect");
});

test("failed reads and revoked grants disable every export and verification action", async () => {
  const f = fixture(); f.setCurrent(profile({ status: "ready" }));
  await f.view.load(); await f.view.openProfile(id);
  f.setCurrent(profile({ status: "revoked" })); await f.byId("local-client-check").handlers.click();
  for (const action of ["copy", "download", "copy-command", "copy-verification", "copy-guidance"]) {
    assert.equal(f.byId("local-client-" + action).disabled, true);
    await f.byId("local-client-" + action).handlers.click();
  }
  assert.deepEqual(f.copied, []); assert.deepEqual(f.downloaded, []);
  assert.equal(f.byId("local-client-config").value, "");
  assert.equal(f.byId("local-client-config-section").hidden, true);
});

test("an unknown saved target falls back to generic, and malformed exports never copy", async () => {
  const f = fixture({ preferences: new Map([[id, "removed-client"]]), request: async path => {
    if (path.endsWith("/config")) return { client: profile({ status: "ready" }), config: { mcpServers: {} } };
    if (path === "/api/local-clients") return { available: true, clients: [], scopeOptions: scopes };
    return profile({ status: "ready" });
  } });
  await f.view.load(); await f.view.openProfile(id);
  assert.equal(f.byId("local-client-target").value, "generic");
  assert.equal(f.byId("local-client-config-error").hidden, false);
  assert.equal(f.byId("local-client-copy").disabled, true);
  await f.byId("local-client-copy").handlers.click(); assert.deepEqual(f.copied, []);
});
