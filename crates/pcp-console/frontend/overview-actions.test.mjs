import assert from "node:assert/strict";
import test from "node:test";
import {
  createOverviewActions,
  summarizeMaintenanceActions,
  summarizeClientApprovals,
  summarizeContextActions,
} from "../src/overview-actions.js";

const NOW = Date.parse("2026-10-22T09:00:00Z");
const before = "2026-10-21T09:00:00Z", after = "2026-10-23T09:00:00Z";
const maintenance = (automation = {}, extra = {}) => ({ available: true, enabled: true, automation: { state: "waiting", ...automation }, ...extra });
const review = (id, queue = "human", extra = {}) => ({ candidateId: id, status: "pending", queue: { state: queue }, ...extra });
const enrollment = (pending = []) => ({ result: { status: "snapshot", pending, registrations: [] } });
const candidate = (extra = {}) => ({ candidateId: "candidate_one", version: 1, status: "pending", expiresAt: after, ...extra });
const synthesis = (extra = {}) => ({ synthesisId: "synthesis_one", status: "pending", maturity: "ready", candidates: [{ candidateId: "candidate_one", version: 1 }], outputs: [{ title: "A proposed memory" }], ...extra });
const inbox = (syntheses = [], organization = {}) => ({ candidates: [candidate()], syntheses, organization: { available: true, automaticReviewEnabled: true, ...organization } });

test("maintenance counts Runtime human routing, excluding background, snoozed and completed reviews", () => {
  const reviews = [
    ...Array.from({ length: 5 }, (_, index) => review(`human_${index}`)),
    ...Array.from({ length: 22 }, (_, index) => review(`budget_${index}`, "waiting_budget")),
    ...["automatic", "waiting_accumulation", "paused", "stale"].map(queue => review(queue, queue)),
    review("done", "human", { status: "accepted" }),
    review("snoozed", "human", { snoozedUntil: after }),
  ];
  assert.deepEqual(summarizeMaintenanceActions(maintenance({ pendingReviewCount: reviews.length }), { reviews }, NOW), { available: true, reviewCount: 5, issueCount: 0 });
  assert.equal(summarizeMaintenanceActions(maintenance(), { reviews: [review("legacy", "human", { queue: null })] }, NOW).reviewCount, 1);
  assert.throws(() => summarizeMaintenanceActions(maintenance(), { reviews: [review("unknown", "future_route")] }, NOW), /Unknown.*routing/);
  assert.throws(() => summarizeMaintenanceActions(maintenance({ state: "unknown" }), { reviews: [] }, NOW), /Unexpected/);
});

test("only a currently stopped or stale enabled maintenance controller is an intervention", () => {
  const history = { jobIssues: [{ automaticRetryExhausted: true, reason: "waiting for new evidence" }], jobIssueHistory: [{ disposition: "failed" }], archivedJobIssueCount: 90 };
  for (const automation of [
    { state: "waiting", ...history },
    { state: "failed", lastError: "request timeout", nextWakeAt: after, ...history },
    { state: "failed", lastError: "PCP total maintenance budget exhausted", ...history },
  ]) assert.equal(summarizeMaintenanceActions(maintenance(automation), { reviews: [] }, NOW).issueCount, 0);
  assert.equal(summarizeMaintenanceActions(maintenance({ state: "stale", ...history }), { reviews: [] }, NOW).issueCount, 1);
  assert.equal(summarizeMaintenanceActions(maintenance({ state: "failed", lastError: "controller stopped" }), { reviews: [] }, NOW).issueCount, 1);
  assert.equal(summarizeMaintenanceActions(maintenance({ state: "failed", lastError: "controller stopped", nextWakeAt: before }), { reviews: [] }, NOW).issueCount, 1);
  assert.equal(summarizeMaintenanceActions(maintenance({ state: "stale" }, { enabled: false }), { reviews: [] }, NOW).issueCount, 0);
  assert.deepEqual(summarizeMaintenanceActions({ available: false }), { available: false });
  assert.throws(() => summarizeMaintenanceActions(maintenance(), {}), /Unexpected/);
});

test("pending client approvals use snapshot state and exclude expired requests", () => {
  assert.equal(summarizeClientApprovals(enrollment([
    { request_id: "current", expires_at: after }, { request_id: "expired", expires_at: before }, { request_id: "legacy" },
  ]), NOW).reviewCount, 2);
  for (const response of [{}, { result: { status: "disabled", pending: [] } }, { result: { status: "snapshot" } }]) {
    assert.throws(() => summarizeClientApprovals(response, NOW), /Unexpected/);
  }
  assert.throws(() => summarizeClientApprovals(enrollment([{ expires_at: "invalid" }]), NOW), /Invalid/);
});

test("the live-style inbox with completed history and an evidence wait has no human task", () => {
  const groups = [
    ...Array.from({ length: 26 }, (_, index) => synthesis({ synthesisId: `completed_${index}`, status: "promoted", automaticReview: { state: "written" } })),
    synthesis({ synthesisId: "waiting", automaticReview: { state: "accumulating" } }),
  ];
  const snapshot = inbox(groups, { error: "old budget failure", nextAttemptAt: after });
  snapshot.candidates.push(candidate({ candidateId: "deferred", status: "deferred" }));
  assert.deepEqual(summarizeContextActions(snapshot, NOW), { available: true, reviewCount: 0, issueCount: 0 });
});

test("context distinguishes explicit human input, operator-only ready plans, and active blockers", () => {
  const groups = [
    synthesis({ synthesisId: "answer", automaticReview: { state: "needs_input" } }),
    synthesis({ synthesisId: "held", automaticReview: { state: "needs_review" } }),
    synthesis({ synthesisId: "interrupted_write", status: "promoting", automaticReview: { state: "write_interrupted" } }),
    synthesis({ synthesisId: "stopped_history", status: "interrupted", automaticReview: { state: "write_interrupted" } }),
    ...["running", "waiting_budget", "approved", "applying", "stale", "no_change"].map(state => synthesis({ synthesisId: state, automaticReview: { state } })),
  ];
  assert.deepEqual(summarizeContextActions(inbox(groups), NOW), { available: true, reviewCount: 1, issueCount: 2 });
  assert.equal(summarizeContextActions(inbox([synthesis()], { automaticReviewEnabled: true }), NOW).reviewCount, 0);
  for (const enabled of [false, null]) {
    assert.equal(summarizeContextActions(inbox([synthesis(), synthesis({ maturity: "accumulating" })], { automaticReviewEnabled: enabled }), NOW).reviewCount, 1);
  }
});

test("obsolete, expired and snoozed context evidence cannot become current tasks", () => {
  for (const evidence of [candidate({ version: 2 }), candidate({ expiresAt: before }), candidate({ snoozedUntil: after }), candidate({ status: "rejected" })]) {
    const snapshot = inbox([synthesis({ automaticReview: { state: "needs_input" } })]); snapshot.candidates = [evidence];
    assert.equal(summarizeContextActions(snapshot, NOW).reviewCount, 0);
  }
  assert.throws(() => summarizeContextActions(inbox([synthesis({ automaticReview: { state: "unknown_state" } })]), NOW), /Unknown/);
  assert.throws(() => summarizeContextActions(inbox([synthesis({ status: "unknown_state" })]), NOW), /Unknown/);
  assert.throws(() => summarizeContextActions({}), /Unexpected/);
});

class Node {
  constructor(tag, document) {
    this.tagName = tag.toUpperCase(); this.ownerDocument = document; this.children = []; this.attributes = {}; this.handlers = {};
    this.dataset = {}; this.textContent = ""; this.className = ""; this.hidden = false; this.disabled = false;
    this.classList = {
      add: value => this.classList.toggle(value, true),
      toggle: (value, enabled) => { const classes = new Set(this.className.split(/\s+/).filter(Boolean)); if (enabled) classes.add(value); else classes.delete(value); this.className = [...classes].join(" "); },
    };
  }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener(name, callback) { this.handlers[name] = callback; }
  focus() { this.ownerDocument.activeElement = this; }
}
const visibleText = node => node.hidden ? "" : [node.textContent, ...node.children.map(visibleText)].join(" ");
const walk = node => [node, ...node.children.flatMap(walk)];
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(resolve => setImmediate(resolve));

function fixture({ request, openView = async () => {}, openClientAccess = async () => {} } = {}) {
  let locale = "en", nextTimer = 1;
  const timers = new Map(), listeners = new Map(), calls = [];
  const document = {
    hidden: false,
    createElement: tag => new Node(tag, document),
    addEventListener: (name, callback) => listeners.set(name, callback),
    removeEventListener: (name, callback) => { if (listeners.get(name) === callback) listeners.delete(name); },
    defaultView: {
      setTimeout: (callback, duration) => { const id = nextTimer++; timers.set(id, { callback, duration }); return id; },
      clearTimeout: id => timers.delete(id),
    },
  };
  const root = new Node("section", document);
  const api = async (path, options) => {
    calls.push(path);
    if (request) return request(path, options);
    return ({ "/api/maintenance": maintenance(), "/api/maintenance/reviews": { reviews: Array.from({ length: 5 }, (_, index) => review(`human_${index}`)) }, "/api/enrollment": enrollment(), "/api/context-hub/inbox": inbox() })[path];
  };
  const view = createOverviewActions({ root, request: api, language: () => locale, openView, openClientAccess, now: () => NOW, formatTime: value => value });
  const card = name => walk(root).find(node => node.dataset.overviewSource === name);
  const child = (name, cls) => walk(card(name)).find(node => node.className.split(" ").includes(cls));
  return { view, root, card, child, document, calls, timers, listeners, locale: value => { locale = value; view.render(); } };
}

test("the component uses only required summary endpoints and keeps controls stable across refresh/locale", async () => {
  const { view, root, card, child, calls, document, locale } = fixture();
  assert.match(visibleText(root), /Not checked/); assert.doesNotMatch(visibleText(root), /No action required/);
  const control = child("maintenance", "overview-action-button"); control.focus();
  await view.load();
  assert.deepEqual([...calls].sort(), ["/api/context-hub/inbox", "/api/enrollment", "/api/maintenance", "/api/maintenance/reviews"].sort());
  assert.match(visibleText(card("maintenance")), /5 human decisions/);
  assert.match(visibleText(card("clients")), /No pending approvals/);
  locale("zh");
  assert.match(visibleText(root), /待你处理.*5 项人工处理/);
  assert.equal(child("maintenance", "overview-action-button"), control); assert.equal(document.activeElement, control);
  assert.equal(root.attributes["aria-busy"], "false");
  assert.equal(root.attributes["aria-labelledby"], "overview-actions-title");
  view.destroy();
});

test("loading and a failed required check do not report zero or all clear; retry stays local", async () => {
  const wait = deferred(); let broken = true;
  const { view, root, card, child, locale, calls } = fixture({ request: path => {
    if (path === "/api/maintenance") return { available: false };
    if (path === "/api/context-hub/inbox") return inbox();
    if (path === "/api/enrollment") return broken ? wait.promise : enrollment([{ request_id: "new", expires_at: after }]);
    throw new Error(`Unexpected endpoint ${path}`);
  } });
  const pending = view.load(); await tick();
  assert.match(visibleText(card("clients")), /Checking/); assert.doesNotMatch(visibleText(card("clients")), /No pending approvals/);
  locale("zh"); wait.reject(new Error("offline")); await pending;
  assert.match(visibleText(root), /部分检查尚不可用/);
  assert.match(visibleText(card("clients")), /检查失败/); assert.doesNotMatch(visibleText(card("clients")), /没有待批准/);
  assert.match(visibleText(card("maintenance")), /暂不可用/);
  assert.ok(!calls.includes("/api/maintenance/reviews"));
  broken = false; const previousCalls = calls.length;
  await child("clients", "compact-button").handlers.click();
  assert.equal(calls.length, previousCalls + 1); assert.equal(calls.at(-1), "/api/enrollment");
  assert.match(visibleText(card("clients")), /1 项待批准申请/);
  assert.match(visibleText(root), /部分检查尚不可用/);
  view.destroy();
});

test("a failed refresh hides the previous count and preserves its clearly labelled timestamp", async () => {
  let fail = false;
  const { view, card } = fixture({ request: async path => {
    if (path === "/api/maintenance") return maintenance();
    if (path === "/api/maintenance/reviews") { if (fail) throw new Error("offline"); return { reviews: [review("one")] }; }
    if (path === "/api/enrollment") return enrollment();
    return inbox();
  } });
  await view.load(); fail = true; await view.load();
  assert.match(visibleText(card("maintenance")), /Could not check.*Last successful check:/);
  assert.doesNotMatch(visibleText(card("maintenance")), /1 human decision/);
  view.destroy();
});

test("a stalled read has bounded recovery and aborts its request without reporting zero", async () => {
  const stalled = deferred(); let signal, retry = false;
  const { view, card, child, timers } = fixture({ request: (path, options) => {
    if (path === "/api/enrollment") { signal = options.signal; return retry ? enrollment([{ request_id: "new", expires_at: after }]) : stalled.promise; }
    if (path === "/api/maintenance") return maintenance();
    if (path === "/api/maintenance/reviews") return { reviews: [] };
    return inbox();
  } });
  const pending = view.load(); await tick();
  const timeout = [...timers.values()].find(timer => timer.duration === 20_000);
  assert.ok(timeout); timeout.callback(); await pending;
  assert.equal(signal.aborted, true);
  assert.match(visibleText(card("clients")), /Could not check/);
  assert.doesNotMatch(visibleText(card("clients")), /No pending approvals/);
  stalled.resolve(enrollment()); await tick();
  assert.match(visibleText(card("clients")), /Could not check/);
  retry = true; await child("clients", "compact-button").handlers.click();
  assert.match(visibleText(card("clients")), /1 pending approval/);
  view.destroy();
});

test("a superseded source success or failure cannot overwrite the latest snapshot", async () => {
  for (const rejectOlder of [false, true]) {
    const old = deferred(), latest = deferred(); let attempt = 0;
    const { view, card } = fixture({ request: path => {
      if (path === "/api/enrollment") return (++attempt === 1 ? old : latest).promise;
      if (path === "/api/maintenance") return maintenance();
      if (path === "/api/maintenance/reviews") return { reviews: [] };
      return inbox();
    } });
    const first = view.load(); await tick(); const second = view.load({ force: true }); await tick();
    latest.resolve(enrollment([{ request_id: "new", expires_at: after }])); await second;
    if (rejectOlder) old.reject(new Error("old failure")); else old.resolve(enrollment());
    await first;
    assert.match(visibleText(card("clients")), /1 pending approval/);
    assert.doesNotMatch(visibleText(card("clients")), /old failure|Could not check|No pending approvals/);
    view.destroy();
  }
});

test("active-visible refresh stops off-page, resumes on visibility, and disposal rejects late results", async () => {
  const { view, calls, timers, listeners, document, root } = fixture();
  view.setActive(true); await tick();
  assert.equal(timers.size, 1); assert.equal([...timers.values()][0].duration, 30_000);
  const firstCount = calls.length;
  document.hidden = true; listeners.get("visibilitychange")(); assert.equal(timers.size, 0);
  document.hidden = false; listeners.get("visibilitychange")(); await tick();
  assert.ok(calls.length > firstCount); assert.equal(timers.size, 1);
  view.setActive(false); assert.equal(timers.size, 0);
  view.destroy(); assert.equal(listeners.size, 0);
  const oldText = visibleText(root); await view.load(); assert.equal(visibleText(root), oldText);
});

test("leaving and reentering invalidates a read from before an off-page operator decision", async () => {
  const old = deferred(); let clients = 0;
  const { view, card } = fixture({ request: path => {
    if (path === "/api/enrollment") return ++clients === 1 ? old.promise : enrollment([{ request_id: "new", expires_at: after }]);
    if (path === "/api/maintenance") return maintenance();
    if (path === "/api/maintenance/reviews") return { reviews: [] };
    return inbox();
  } });
  view.setActive(true); await tick(); view.setActive(false); view.setActive(true); await tick();
  old.resolve(enrollment()); await tick();
  assert.match(visibleText(card("clients")), /1 pending approval/);
  view.destroy();
});

test("destination buttons own busy and retry feedback without making permission writes", async () => {
  const opening = deferred(), targets = []; let fail = true;
  const { view, child, card } = fixture({ openView: async target => { targets.push(target); if (fail) await opening.promise; }, openClientAccess: async () => { targets.push("clients"); } });
  const control = child("maintenance", "overview-action-button"), pending = control.handlers.click();
  assert.equal(control.disabled, true); assert.match(control.textContent, /Opening/);
  opening.reject(new Error("destination offline")); await pending;
  assert.equal(control.disabled, false); assert.match(visibleText(card("maintenance")), /Could not open this view/);
  fail = false; await control.handlers.click();
  assert.doesNotMatch(visibleText(card("maintenance")), /Could not open/);
  await child("clients", "overview-action-button").handlers.click();
  await child("context", "overview-action-button").handlers.click();
  assert.deepEqual(targets, ["maintenance", "maintenance", "clients", "context-hub"]);
  view.destroy();
});
