import { partitionReviewQueues } from "./maintenance-review-session.js";
import { maintenanceFailureKind } from "./maintenance-diagnostics.js";

const REVIEW_QUEUES = new Set(["human", "automatic", "waiting_budget", "waiting_accumulation", "paused", "stale"]);
const AUTOMATION_STATES = new Set(["not_started", "waiting", "running", "failed", "stale"]);
const SYNTHESIS_HISTORY = new Set(["promoted", "interrupted", "superseded"]);
const AUTOMATIC_WAIT = new Set(["running", "waiting_budget", "approved", "applying", "written", "accumulating", "stale", "no_change"]);
const SOURCE_NAMES = ["maintenance", "clients", "context"];
const REFRESH_INTERVAL = 30_000;
const READ_TIMEOUT = 20_000;
const future = (value, now) => value && Number.isFinite(Date.parse(value)) && Date.parse(value) > now;

// Review routing belongs to Runtime. Pending proposal totals include automatic
// work and are deliberately not used as a count of human decisions.
export function summarizeMaintenanceActions(status, response, now = Date.now()) {
  if (status?.available === false) return { available: false };
  if (status?.available !== true || !AUTOMATION_STATES.has(status.automation?.state) || !Array.isArray(response?.reviews)) throw new Error("Unexpected maintenance summary response");
  if (response.reviews.some(item => !["pending", "accepted", "rejected", "deferred", "suppressed", "stale"].includes(item.status))) throw new Error("Unknown maintenance review state");
  const reviews = response.reviews.filter(item => item.status === "pending" && !future(item.snoozedUntil, now));
  if (reviews.some(item => item.queue && !REVIEW_QUEUES.has(item.queue.state))) throw new Error("Unknown maintenance review routing");
  const human = partitionReviewQueues(reviews).human;
  const automation = status.automation;
  // Retryable failures, exhausted evidence waits and archived job failures are
  // background diagnostics. A stopped/stale enabled controller needs inspection.
  const stopped = status.enabled === true && (automation.state === "stale"
    || (automation.state === "failed" && automation.lastError && !future(automation.nextWakeAt, now)
      && maintenanceFailureKind(automation.lastError) !== "waiting_budget"));
  return { available: true, reviewCount: human.length, issueCount: stopped ? 1 : 0 };
}

export function summarizeClientApprovals(response, now = Date.now()) {
  const result = response?.result;
  if (result?.status !== "snapshot" || !Array.isArray(result.pending)) throw new Error("Unexpected client approval response");
  if (result.pending.some(item => item.expires_at && !Number.isFinite(Date.parse(item.expires_at)))) throw new Error("Invalid client request expiry");
  return { available: true, reviewCount: result.pending.filter(item => !item.expires_at || future(item.expires_at, now)).length, issueCount: 0 };
}

// Inbox summaries contain routing and receipts, without full review evidence.
// Raw candidates, deferred evidence and completed syntheses are not human tasks.
export function summarizeContextActions(inbox, now = Date.now()) {
  if (!Array.isArray(inbox?.candidates) || !Array.isArray(inbox.syntheses) || !inbox.organization) throw new Error("Unexpected context inbox response");
  let reviewCount = 0, issueCount = 0;
  const candidates = new Map(inbox.candidates.map(item => [item.candidateId, item]));
  for (const group of inbox.syntheses) {
    if (SYNTHESIS_HISTORY.has(group.status)) continue;
    if (!["pending", "promoting"].includes(group.status)) throw new Error("Unknown context synthesis state");
    // Ignore an obsolete or snoozed plan even if an older server includes it.
    if (!Array.isArray(group.candidates) || !group.candidates.length) throw new Error("Missing context synthesis evidence");
    const refs = group.candidates;
    if (refs.some(ref => {
      const item = candidates.get(ref.candidateId);
      return !item || item.version !== ref.version || !["pending", "deferred", "promoting"].includes(item.status)
        || future(item.snoozedUntil, now) || (item.expiresAt && !future(item.expiresAt, now));
    })) continue;
    const automatic = group.automaticReview?.state;
    if (automatic === "needs_input") reviewCount++;
    else if (["needs_review", "write_interrupted"].includes(automatic)) issueCount++;
    else if (automatic && !AUTOMATIC_WAIT.has(automatic)) throw new Error("Unknown automatic context review state");
    else if (!automatic && group.status === "pending" && group.maturity === "ready" && group.outputs?.length
      && inbox.organization.automaticReviewEnabled !== true) reviewCount++;
  }
  return { available: true, reviewCount, issueCount };
}

const COPY = {
  title: ["Needs your attention", "待你处理"],
  description: ["Review decisions and blocked work in their own views.", "在原有界面处理待定事项和需要介入的异常。"],
  refresh: ["Refresh checks", "刷新待办"],
  checking: ["Checking…", "正在检查…"],
  idle: ["Not checked", "尚未检查"],
  unavailable: ["Unavailable", "暂不可用"],
  failed: ["Could not check", "检查失败"],
  incomplete: ["Some checks are unavailable. Retry or open the related view.", "部分检查尚不可用，可重试或打开对应界面查看。"],
  clear: ["No action required in the latest checks.", "最近一次检查没有发现需要处理的事项。"],
  pending: ["Open the related view to inspect and act.", "打开对应界面查看详情并处理。"],
  retry: ["Retry check", "重新检查"],
  details: ["Check details", "检查详情"],
  opening: ["Opening…", "正在打开…"],
  openFailed: ["Could not open this view. Try again.", "未能打开此界面，请重试。"],
  lastChecked: ["Checked {time}", "检查于 {time}"],
  lastSuccessful: ["Last successful check: {time}", "上次成功检查：{time}"],
  maintenance: ["Maintenance decisions", "维护审阅"],
  clients: ["Client approvals", "客户端批准"],
  context: ["Context decisions", "候选审阅"],
  maintenanceOpen: ["Review maintenance", "打开维护审阅"],
  clientsOpen: ["Review client access", "查看客户端申请"],
  contextOpen: ["Open context inbox", "打开暂存与近况"],
  noReviews: ["No human decisions flagged", "没有标记为人工处理的事项"],
  reviews: ["{count} human decisions", "{count} 项人工处理"],
  review: ["1 human decision", "1 项人工处理"],
  noClients: ["No pending approvals", "没有待批准的申请"],
  approvals: ["{count} pending approvals", "{count} 项待批准申请"],
  approval: ["1 pending approval", "1 项待批准申请"],
  issues: ["{count} blocked items to inspect", "{count} 项异常需要查看"],
  issue: ["1 blocked item to inspect", "1 项异常需要查看"],
  sourceNote: ["Automatic work, evidence waits and completed history are excluded.", "不计入后台执行、等待证据和已完成历史。"],
};

// Owns the read snapshot, per-source recovery, active refresh and translation.
// The app owns navigation and every approval/write action in the destination view.
export function createOverviewActions({ root, request, language, openView, openClientAccess, formatTime, now = Date.now }) {
  const document = root.ownerDocument;
  const host = document.defaultView;
  const state = Object.fromEntries(SOURCE_NAMES.map(name => [name, { phase: "idle", generation: 0, pending: null, summary: null, checkedAt: null, error: "", opening: false, actionError: "" }]));
  const nodes = {};
  let active = false, destroyed = false, timer = null;
  const text = (key, params = {}) => Object.entries(params).reduce((value, [name, replacement]) => value.replace(`{${name}}`, replacement), COPY[key][language() === "zh" ? 1 : 0]);
  const node = (tag, className = "") => { const item = document.createElement(tag); item.className = className; return item; };
  const button = (className, handler) => {
    const item = node("button", className); item.type = "button"; item.addEventListener("click", handler); return item;
  };
  root.classList.add("overview-actions");
  root.setAttribute("aria-labelledby", "overview-actions-title");
  const heading = node("div", "overview-action-heading"), copy = node("div");
  const title = node("h2"); title.id = "overview-actions-title";
  const description = node("p", "muted");
  const refresh = button("compact-button overview-action-refresh", () => load({ force: true }));
  copy.append(title, description); heading.append(copy, refresh);
  const status = node("p", "overview-action-state"); status.setAttribute("role", "status"); status.setAttribute("aria-live", "polite");
  const grid = node("div", "overview-action-grid");
  for (const name of SOURCE_NAMES) {
    const card = node("article", "overview-action-card"); card.dataset.overviewSource = name;
    const label = node("h3"), count = node("p", "overview-action-count"), issue = node("p", "overview-action-issue");
    const actionError = node("p", "overview-action-error"); actionError.setAttribute("role", "alert");
    const detail = node("details", "overview-action-error");
    const detailTitle = node("summary"), detailMessage = node("pre"); detail.append(detailTitle, detailMessage);
    const footer = node("div", "overview-action-footer"), checked = node("span", "muted");
    const open = button("secondary-button overview-action-button", () => openSource(name));
    const retry = button("compact-button", () => load({ source: name, force: true }));
    footer.append(open, retry, checked); card.append(label, count, issue, actionError, detail, footer); grid.append(card);
    nodes[name] = { card, label, count, issue, actionError, detail, detailTitle, detailMessage, checked, open, retry };
  }
  const note = node("p", "muted overview-action-note");
  root.replaceChildren(heading, status, grid, note);

  function render() {
    if (destroyed) return;
    const values = Object.values(state), loading = values.some(item => item.phase === "loading");
    const complete = values.every(item => item.phase === "ready");
    const hasActions = values.some(item => item.phase === "ready" && (item.summary.reviewCount || item.summary.issueCount));
    root.setAttribute("aria-busy", String(loading));
    title.textContent = text("title"); description.textContent = text("description");
    refresh.textContent = text(loading ? "checking" : "refresh"); refresh.disabled = loading;
    status.textContent = text(loading ? "checking" : !complete ? "incomplete" : hasActions ? "pending" : "clear");
    note.textContent = text("sourceNote");
    for (const name of SOURCE_NAMES) {
      const item = state[name], ui = nodes[name], ready = item.phase === "ready";
      ui.card.setAttribute("aria-busy", String(item.phase === "loading" || item.opening));
      ui.card.classList.toggle("has-actions", ready && Boolean(item.summary.reviewCount || item.summary.issueCount));
      ui.label.textContent = text(name);
      const countKey = ready && (name === "clients"
        ? item.summary.reviewCount === 1 ? "approval" : item.summary.reviewCount ? "approvals" : "noClients"
        : item.summary.reviewCount === 1 ? "review" : item.summary.reviewCount ? "reviews" : "noReviews");
      ui.count.textContent = ready
        ? text(countKey, { count: item.summary.reviewCount })
        : text(({ idle: "idle", loading: "checking", unavailable: "unavailable", error: "failed" })[item.phase]);
      ui.issue.hidden = !ready || !item.summary.issueCount;
      ui.issue.textContent = ready ? text(item.summary.issueCount === 1 ? "issue" : "issues", { count: item.summary.issueCount }) : "";
      ui.actionError.hidden = !item.actionError; ui.actionError.textContent = item.actionError ? text("openFailed") : "";
      ui.detail.hidden = item.phase !== "error"; ui.detailTitle.textContent = text("details"); ui.detailMessage.textContent = item.error;
      ui.open.textContent = text(item.opening ? "opening" : `${name}Open`); ui.open.disabled = item.opening;
      ui.retry.hidden = !["error", "unavailable"].includes(item.phase); ui.retry.textContent = text("retry");
      ui.checked.hidden = !item.checkedAt;
      const time = item.checkedAt && (formatTime ? formatTime(item.checkedAt) : new Date(item.checkedAt).toLocaleString(language() === "zh" ? "zh-CN" : "en"));
      ui.checked.textContent = time ? text(ready ? "lastChecked" : "lastSuccessful", { time }) : "";
    }
  }

  async function readSource(name, current, signal) {
    if (name === "clients") return summarizeClientApprovals(await request("/api/enrollment", { signal }), now());
    if (name === "context") return summarizeContextActions(await request("/api/context-hub/inbox", { signal }), now());
    const maintenance = await request("/api/maintenance", { signal });
    if (!current()) return null;
    if (maintenance?.available === false) return { available: false };
    return summarizeMaintenanceActions(maintenance, await request("/api/maintenance/reviews", { signal }), now());
  }

  function loadSource(name, force) {
    const item = state[name];
    if (item.pending && !force) return item.pending;
    const generation = ++item.generation;
    item.cancel?.();
    const current = () => !destroyed && item.generation === generation;
    const controller = new AbortController();
    let rejectWait;
    const expired = new Promise((_, reject) => { rejectWait = reject; });
    const timeout = host.setTimeout(() => {
      const error = new Error("Summary check timed out; retry the check");
      rejectWait(error); controller.abort(error);
    }, READ_TIMEOUT);
    item.cancel = () => { host.clearTimeout(timeout); controller.abort(); rejectWait(new Error("Summary check cancelled")); };
    item.phase = "loading"; item.error = "";
    const pending = Promise.resolve().then(async () => {
      try {
        const summary = await Promise.race([readSource(name, current, controller.signal), expired]);
        if (!current()) return;
        item.summary = summary; item.phase = summary.available ? "ready" : "unavailable";
        if (summary.available) item.checkedAt = new Date(now()).toISOString();
      } catch (error) {
        if (!current()) return;
        item.phase = "error"; item.error = String(error?.message || error).slice(0, 1000);
      } finally {
        host.clearTimeout(timeout);
        if (current()) { item.pending = null; item.cancel = null; render(); }
      }
    });
    item.pending = pending;
    return pending;
  }

  function load({ force = false, source } = {}) {
    if (destroyed) return Promise.resolve();
    const names = source ? SOURCE_NAMES.filter(name => name === source) : SOURCE_NAMES;
    const pending = Promise.all(names.map(name => loadSource(name, force)));
    render();
    return pending;
  }

  async function openSource(name) {
    const item = state[name];
    if (item.opening || destroyed) return;
    item.opening = true; item.actionError = ""; render();
    try { await (name === "clients" ? openClientAccess() : openView(name === "maintenance" ? "maintenance" : "context-hub")); }
    catch (error) { if (!destroyed) item.actionError = String(error?.message || error); }
    finally { item.opening = false; render(); }
  }

  function clearTimer() { if (timer !== null) host.clearTimeout(timer); timer = null; }
  function schedule() {
    clearTimer();
    if (destroyed || !active || document.hidden) return;
    timer = host.setTimeout(async () => { timer = null; await load(); schedule(); }, REFRESH_INTERVAL);
  }
  function visibilityChanged() {
    clearTimer();
    if (active && !document.hidden && !destroyed) void load({ force: true }).then(schedule);
  }
  function setActive(value) {
    if (destroyed || active === Boolean(value)) return;
    active = Boolean(value); clearTimer();
    if (active && !document.hidden) void load().then(schedule);
    else if (!active) {
      // Reads from before an off-page decision cannot become the next snapshot.
      for (const item of Object.values(state)) {
        item.generation++; item.cancel?.(); item.cancel = null; item.pending = null;
        if (item.phase === "loading") item.phase = item.summary ? item.summary.available ? "ready" : "unavailable" : "idle";
      }
      render();
    }
  }
  function destroy() {
    destroyed = true; active = false; clearTimer();
    for (const item of Object.values(state)) { item.generation++; item.cancel?.(); item.cancel = null; }
    document.removeEventListener("visibilitychange", visibilityChanged);
  }
  document.addEventListener("visibilitychange", visibilityChanged);
  render();
  return { load, render, setActive, destroy };
}
