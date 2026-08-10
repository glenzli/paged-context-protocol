import { createPageInspector } from "/page-inspector.js";
import { createQualityView } from "/quality-view.js";
import { createHealthView } from "/health-view.js";
import { createRetentionView } from "/retention-view.js";

const PAGE_LIMIT = 20;
const ACCESS_LIMIT = 50;
const state = {
  overview: null,
  activeView: "overview",
  pages: { loaded: false, busy: false, cursor: null, count: 0 },
  access: { loaded: false, busy: false, cursor: null, count: 0, events: [] },
  enrollment: { available: false, seenPending: new Set() },
};
const byId = (id) => document.getElementById(id);

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function formatNumber(value) {
  return new Intl.NumberFormat().format(value || 0);
}

function formatSize(chars) {
  if (chars < 1000) return `${chars} chars`;
  if (chars < 1_000_000) return `${(chars / 1000).toFixed(1)}k chars`;
  return `${(chars / 1_000_000).toFixed(2)}M chars`;
}

function formatTime(value) {
  if (!value) return "-";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function projectionLabel(value) {
  return ({
    summary: "Summary",
    payload: "Content",
    facets: "Metadata",
    relations: "Relations",
  })[value] || value || "-";
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { Accept: "application/json", ...(options.headers || {}) },
  });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try { message = (await response.json()).error || message; } catch (_) {}
    throw new Error(message);
  }
  return response.status === 204 ? null : response.json();
}

async function enrollmentMutation(path) {
  return api(path, {
    method: "POST",
    headers: { "X-PCP-Console": "1" },
  });
}

function enrollmentAccessLabel(access) {
  const scopes = access.scopes.join(", ");
  return `${access.mode} / ${scopes}${access.allow_cross_scope_derivation ? " / cross-scope derivation" : ""}`;
}

function enrollmentIdentity(client) {
  const principal = client.principal;
  return principal.displayName || principal.principalId;
}

function enrollmentRow(item, pending) {
  const row = element("article", "enrollment-row");
  const identity = element("div", "enrollment-identity");
  identity.append(
    element("strong", "", enrollmentIdentity(item.client)),
    element("span", "mono muted", item.client.principal.principalId),
    element("span", "muted", enrollmentAccessLabel(pending ? item.requested_access : item.approved_access)),
  );
  const actions = element("div", "enrollment-actions");
  if (pending) {
    const reject = element("button", "", "Reject");
    reject.type = "button";
    reject.addEventListener("click", async () => {
      reject.disabled = true;
      try {
        await enrollmentMutation(`/api/enrollment/requests/${encodeURIComponent(item.request_id)}/reject`);
        await loadEnrollment({ autoOpen: false });
      } catch (error) {
        reject.disabled = false;
        showError(error);
      }
    });
    const approve = element("button", "primary-button", "Approve");
    approve.type = "button";
    approve.addEventListener("click", async () => {
      approve.disabled = true;
      try {
        await enrollmentMutation(`/api/enrollment/requests/${encodeURIComponent(item.request_id)}/approve`);
        await loadEnrollment({ autoOpen: false });
      } catch (error) {
        approve.disabled = false;
        showError(error);
      }
    });
    actions.append(reject, approve);
  } else {
    const revoke = element("button", "danger-button", "Revoke");
    revoke.type = "button";
    revoke.addEventListener("click", async () => {
      revoke.disabled = true;
      try {
        await enrollmentMutation(`/api/enrollment/registrations/${encodeURIComponent(item.registration_id)}/revoke`);
        await loadEnrollment({ autoOpen: false });
      } catch (error) {
        revoke.disabled = false;
        showError(error);
      }
    });
    actions.append(revoke);
  }
  row.append(identity, actions);
  return row;
}

function renderEnrollment(data, autoOpen) {
  const snapshot = data.result;
  if (snapshot.status !== "snapshot") throw new Error("Unexpected enrollment response");
  state.enrollment.available = true;
  byId("enrollment-open").hidden = false;
  const pending = snapshot.pending || [];
  const registered = snapshot.registrations || [];
  const badge = byId("enrollment-badge");
  badge.textContent = String(pending.length);
  badge.hidden = pending.length === 0;

  const pendingList = byId("enrollment-pending");
  pendingList.replaceChildren(...pending.map((item) => enrollmentRow(item, true)));
  if (pending.length === 0) pendingList.append(element("div", "empty enrollment-empty", "No pending requests"));
  const registeredList = byId("enrollment-registered");
  registeredList.replaceChildren(...registered.map((item) => enrollmentRow(item, false)));
  if (registered.length === 0) registeredList.append(element("div", "empty enrollment-empty", "No approved clients"));

  const unseen = pending.filter((item) => !state.enrollment.seenPending.has(item.request_id));
  if (autoOpen && unseen.length > 0 && !document.querySelector("dialog[open]")) {
    byId("enrollment-dialog").showModal();
  }
  if (byId("enrollment-dialog").open) {
    pending.forEach((item) => state.enrollment.seenPending.add(item.request_id));
  }
}

async function loadEnrollment({ autoOpen = true } = {}) {
  try {
    renderEnrollment(await api("/api/enrollment"), autoOpen);
  } catch (_) {
    if (!state.enrollment.available) byId("enrollment-open").hidden = true;
  }
}

function showError(error) {
  const box = byId("error");
  box.textContent = error.message || String(error);
  box.hidden = false;
  window.setTimeout(() => { box.hidden = true; }, 7000);
}

const pageInspector = createPageInspector({ request: api, showError, formatTime });
const qualityView = createQualityView({
  request: api,
  showError,
  formatNumber,
  openPage: (pageId) => pageInspector.open(pageId),
});
const healthView = createHealthView({ request: api, showError, formatNumber });
const retentionView = createRetentionView({
  request: api,
  showError,
  formatNumber,
  formatTime,
  openPage: (pageId) => pageInspector.open(pageId),
});

function metric(label, value, tone = "") {
  const node = element("div", `metric${tone ? ` tone-${tone}` : ""}`);
  node.append(element("div", "metric-label", label), element("div", "metric-value", value));
  return node;
}

function decisionTone(decision) {
  const value = String(decision || "").toLowerCase();
  if (["allow", "allowed", "granted"].includes(value)) return "allowed";
  if (["deny", "denied", "rejected"].includes(value)) return "denied";
  return "other";
}

function scopeName(namespace) {
  const scope = state.overview?.scopes.find((item) => item.namespace === namespace);
  return scope?.displayName || namespace || "All scopes";
}

function orderedScopes(scopes) {
  const byNamespace = new Map(scopes.map((scope) => [scope.namespace, scope]));
  const children = new Map();
  const roots = [];
  for (const scope of scopes) {
    if (scope.parentNamespace && byNamespace.has(scope.parentNamespace)) {
      const siblings = children.get(scope.parentNamespace) || [];
      siblings.push(scope);
      children.set(scope.parentNamespace, siblings);
    } else {
      roots.push(scope);
    }
  }
  const compare = (left, right) => (left.displayName || left.namespace).localeCompare(right.displayName || right.namespace);
  const output = [];
  const visited = new Set();
  function visit(scope, depth) {
    if (visited.has(scope.namespace)) return;
    visited.add(scope.namespace);
    output.push({ scope, depth });
    (children.get(scope.namespace) || []).sort(compare).forEach((child) => visit(child, depth + 1));
  }
  roots.sort(compare).forEach((scope) => visit(scope, 0));
  scopes.sort(compare).forEach((scope) => visit(scope, 0));
  return output;
}

function renderOverview(data) {
  state.overview = data;
  const connected = data.integrity === "ok";
  byId("connection").textContent = connected ? "Connected" : "Degraded";
  byId("connection").classList.toggle("ready", connected);
  byId("connection").classList.toggle("degraded", !connected);
  byId("headline-pages").textContent = formatNumber(data.pageCount);
  byId("headline-content").textContent = formatSize(data.contentChars);

  byId("metrics").replaceChildren(
    metric("Integrity", data.integrity, connected ? "positive" : "danger"),
    metric("Protocol", data.capabilities.protocolVersion, "info"),
    metric("Runtime PID", data.runtime.pid || "-"),
    metric("Runtime started", formatTime(data.runtime.startedAtUnixMs)),
  );

  byId("scope-rows").replaceChildren(...orderedScopes([...data.scopes]).map(({ scope, depth }) => {
    const row = document.createElement("tr");
    const open = element("button", "quiet-button", "Open");
    open.type = "button";
    open.title = `Browse ${scope.displayName || scope.namespace}`;
    open.addEventListener("click", () => openScope(scope.namespace));
    const action = element("td", "action-cell");
    action.append(open);
    const identity = element("td");
    const scopeCell = element("div", "scope-cell");
    scopeCell.style.setProperty("--scope-depth", depth);
    scopeCell.append(
      element("strong", "", scope.displayName || scope.namespace),
      element("span", "mono muted", scope.namespace),
    );
    if (scope.description) scopeCell.append(element("span", "scope-description", scope.description));
    identity.append(scopeCell);
    row.append(
      identity,
      element("td", "", formatNumber(scope.pageCount)),
      element("td", "", formatTime(scope.updatedAt)),
      action,
    );
    return row;
  }));

  const endpointRows = [
    ["Principal", data.principal.principalId],
    ["Principal type", data.principal.principalType],
    ["Owner", data.ownerId],
    ["Session", data.grants.length ? "active" : "no grants"],
    ["Granted scopes", data.grants.map((grant) => grant.namespace).join(", ")],
  ];
  byId("endpoint-details").replaceChildren(...endpointRows.flatMap(([label, value]) => [
    element("dt", "", label),
    element("dd", "mono", value),
  ]));

  const capabilities = data.capabilities;
  const capabilityRows = [
    ["Revisioned Pages", capabilities.supportsRevisionedPages],
    ["Provenance graph", capabilities.supportsProvenanceGraph],
    ["Consolidation", capabilities.supportsConsolidation],
    ["Retention planning", capabilities.supportsRevisionRetentionPlanning],
    ["Retention leases", capabilities.supportsRevisionRetentionLeases],
    ["Retention collection", capabilities.supportsRevisionRetention],
    ["Durable deletion", capabilities.supportsDurableDeletion],
  ];
  byId("capability-details").replaceChildren(...capabilityRows.flatMap(([label, enabled]) => [
    element("dt", "", label),
    element("dd", enabled ? "capability-yes" : "capability-no", enabled ? "Available" : "Unavailable"),
  ]));

  const selector = byId("scope-filter");
  const current = selector.value;
  const all = element("option", "", "All scopes");
  all.value = "";
  selector.replaceChildren(all, ...data.scopes.map((scope) => {
    const option = element("option", "", scope.displayName || scope.namespace);
    option.value = scope.namespace;
    return option;
  }));
  selector.value = current;
  retentionView.setScopes(data.scopes);
  retentionView.setCapabilities(capabilities);
}

function aggregatePanel(label, entries) {
  const panel = element("section", "aggregate-panel");
  panel.append(element("h3", "", label));
  for (const [name, count] of entries) {
    const row = element("div", "aggregate-row");
    if (label === "Decisions") row.classList.add(`decision-${decisionTone(name)}`);
    row.append(element("span", "mono", name), element("strong", "", formatNumber(count)));
    panel.append(row);
  }
  if (!entries.length) panel.append(element("div", "empty", "No events"));
  return panel;
}

function topCounts(events, select) {
  const counts = new Map();
  for (const event of events) {
    const key = select(event) || "unknown";
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  return [...counts.entries()].sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0])).slice(0, 6);
}

function renderAccessSummary() {
  byId("access-summary").replaceChildren(
    aggregatePanel("Operations", topCounts(state.access.events, (event) => event.operation)),
    aggregatePanel("Principals", topCounts(state.access.events, (event) => event.principal.principalId)),
    aggregatePanel("Decisions", topCounts(state.access.events, (event) => event.decision)),
  );
}

function pageRow(hit) {
  const row = document.createElement("tr");
  const pageCell = element("td");
  const open = element("button", "page-link");
  open.type = "button";
  open.append(
    element("span", "mono", hit.pageId),
    element("span", "snippet", hit.snippet || "No preview"),
  );
  open.addEventListener("click", () => pageInspector.open(hit.pageId));
  pageCell.append(open);
  row.append(
    pageCell,
    element("td", "mono", hit.namespace),
    element("td", "", projectionLabel(hit.matchedProjection)),
    element("td", "", formatTime(hit.observedAt || hit.createdAt)),
  );
  return row;
}

function renderPages(data, append) {
  const rows = byId("page-rows");
  const rendered = data.hits.map(pageRow);
  if (append) rows.append(...rendered);
  else rows.replaceChildren(...rendered);

  state.pages.count = append ? state.pages.count + data.hits.length : data.hits.length;
  state.pages.cursor = data.nextCursor || null;
  state.pages.loaded = true;
  byId("pages-status").textContent = scopeName(byId("scope-filter").value);
  byId("pages-loaded").textContent = `${formatNumber(state.pages.count)} loaded`;
  byId("pages-more").hidden = !state.pages.cursor;

  if (state.pages.count === 0) {
    const row = document.createElement("tr");
    const cell = element("td", "empty", "No pages");
    cell.colSpan = 4;
    row.append(cell);
    rows.replaceChildren(row);
  }
}

function renderAccess(data, append) {
  const rows = byId("access-rows");
  const rendered = data.events.map((event) => {
    const row = document.createElement("tr");
    row.append(
      element("td", "", formatTime(event.occurredAt)),
      element("td", "mono", event.principal.principalId),
      element("td", "mono", event.operation),
      element("td", "mono", event.scopes.join(", ")),
      element("td", `decision-${decisionTone(event.decision)}`, event.decision),
    );
    return row;
  });
  if (append) rows.append(...rendered);
  else rows.replaceChildren(...rendered);
  state.access.events = append ? state.access.events.concat(data.events) : data.events;
  state.access.count = append ? state.access.count + data.events.length : data.events.length;
  state.access.cursor = data.nextCursor || null;
  state.access.loaded = true;
  byId("access-status").textContent = "Audit timeline";
  byId("access-loaded").textContent = `${formatNumber(state.access.count)} loaded`;
  byId("access-more").hidden = !state.access.cursor;
  renderAccessSummary();
}

async function loadOverview() {
  renderOverview(await api("/api/overview"));
}

async function loadPages({ append = false } = {}) {
  if (state.pages.busy) return;
  state.pages.busy = true;
  byId("pages-status").textContent = append ? "Loading more" : "Loading";
  byId("pages-more").disabled = true;
  try {
    const params = new URLSearchParams({ limit: String(PAGE_LIMIT) });
    const query = byId("query").value.trim();
    const scope = byId("scope-filter").value;
    if (query) {
      params.set("q", query);
      params.set("mode", byId("search-mode").value);
    }
    if (scope) params.set("scope", scope);
    if (append && state.pages.cursor) params.set("cursor", state.pages.cursor);
    renderPages(await api(`/api/pages?${params}`), append);
  } catch (error) {
    showError(error);
    byId("pages-status").textContent = "Load failed";
  } finally {
    state.pages.busy = false;
    byId("pages-more").disabled = false;
  }
}

async function loadAccess({ append = false } = {}) {
  if (state.access.busy) return;
  state.access.busy = true;
  byId("access-status").textContent = append ? "Loading more" : "Loading";
  byId("access-more").disabled = true;
  try {
    const params = new URLSearchParams({ limit: String(ACCESS_LIMIT) });
    if (append && state.access.cursor) params.set("cursor", state.access.cursor);
    renderAccess(await api(`/api/access?${params}`), append);
  } catch (error) {
    showError(error);
    byId("access-status").textContent = "Load failed";
  } finally {
    state.access.busy = false;
    byId("access-more").disabled = false;
  }
}

async function activateView(name, { reload = false } = {}) {
  state.activeView = name;
  document.querySelectorAll(".tab").forEach((tab) => tab.classList.toggle("active", tab.dataset.view === name));
  document.querySelectorAll(".view").forEach((view) => view.classList.toggle("active", view.id === `view-${name}`));
  if (name === "pages" && (reload || !state.pages.loaded)) await loadPages();
  if (name === "health") {
    await Promise.all([healthView.load({ reload }), qualityView.load({ reload })]);
  }
  if (name === "retention") await retentionView.load({ reload });
  if (name === "access" && (reload || !state.access.loaded)) await loadAccess();
}

async function openScope(namespace) {
  byId("scope-filter").value = namespace;
  byId("query").value = "";
  state.pages.cursor = null;
  state.pages.count = 0;
  await activateView("pages", { reload: true });
}

async function refresh() {
  try {
    await loadOverview();
    if (state.activeView === "pages") await loadPages();
    if (state.activeView === "health") {
      await Promise.all([
        healthView.load({ reload: true }),
        qualityView.load({ reload: true }),
      ]);
    }
    if (state.activeView === "retention") await retentionView.load({ reload: true });
    if (state.activeView === "access") await loadAccess();
  } catch (error) { showError(error); }
}

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => activateView(tab.dataset.view).catch(showError));
});
byId("refresh").addEventListener("click", refresh);
byId("enrollment-open").addEventListener("click", () => {
  byId("enrollment-dialog").showModal();
  loadEnrollment({ autoOpen: false });
});
byId("enrollment-close").addEventListener("click", () => byId("enrollment-dialog").close());
byId("page-search").addEventListener("submit", (event) => {
  event.preventDefault();
  state.pages.cursor = null;
  state.pages.count = 0;
  loadPages().catch(showError);
});
byId("scope-filter").addEventListener("change", () => {
  state.pages.cursor = null;
  state.pages.count = 0;
  loadPages().catch(showError);
});
byId("pages-more").addEventListener("click", () => loadPages({ append: true }).catch(showError));
byId("access-more").addEventListener("click", () => loadAccess({ append: true }).catch(showError));
byId("health-window").addEventListener("change", () => healthView.load({ reload: true }).catch(showError));

refresh();
loadEnrollment();
window.setInterval(() => loadEnrollment(), 3000);
