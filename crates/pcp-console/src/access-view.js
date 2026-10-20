import { operationLabel, technicalDetails, scopeLabel } from "./console-presentation.js";
// The audit view owns its filters, request generation, pagination and rendering.
export function createAccessView({ api, byId, element, t, formatTime, formatNumber, showError, scopeName = value => value }) {
  let loaded = false, generation = 0, cursor = null, events = [], current = null;
  let scopeNames = {};
  let principalId = "", since = "";
  const knownOperations = new Set(), clientNames = new Map();
  let moreOperationsOpen = false;
  function filters() {
    const params = new URLSearchParams({ limit: "50", view: byId("access-view").value || "requests", includeHealthChecks: String(byId("access-health").checked) });
    if (principalId) params.set("principalId", principalId);
    if (since) params.set("since", since);
    const operation = byId("access-operation").value.trim();
    if (operation) params.set("operation", operation);
    return params;
  }
  function unit() { return t((byId("access-view").value || "requests") === "requests" ? "requests" : "events"); }
  function scopeDetails(event) {
    const scopes = event.scopes;
    const names = Object.fromEntries(scopes.map(scope => [scope, scopeNames[scope] || t("Unknown scope")]));
    const label = (values, tag = "span", className = "") => scopeLabel(element, values, scopeName, tag, className, names);
    const sessionScopes = event.telemetry?.request?.root && event.telemetry.request.internalOperations === 0;
    const details = element("details", "access-scope-details");
    const summary = element("summary", "", t(sessionScopes ? "Session scopes" : "Recorded request scopes"));
    summary.append(element("span", "muted", " · "),
      element("span", "access-scope-count", t("{count} scopes in total").replace("{count}", formatNumber(scopes.length))));
    if (sessionScopes) details.append(element("p", "muted", t("No per-scope storage operations were recorded. These are session permissions, not evidence that every project was read.")));
    const list = element("ul", "access-scope-list");
    for (const scope of scopes) {
      const item = element("li");
      item.append(label([scope]), element("code", "", scope));
      list.append(item);
    }
    details.prepend(summary); details.append(list);
    return details;
  }
  function attachOperations(details, request) {
    const summary = element("div", "muted", `${t("Internal operations")}: ${request.internalOperations} · ${t("Batch reads")}: ${request.batchReads} · ${t("Single reads")}: ${request.singleReads} · ${t("Page visits")}: ${request.pageVisits}`);
    const list = element("div", "access-internal-operations");
    const more = element("button", "compact-button secondary-button", t("Load more")); more.type = "button"; more.hidden = true;
    details.append(summary, element("div", "mono", `${t("Request ID")}: ${request.id}`), list, more);
    let started = false, busy = false, next = null;
    async function loadChildren() {
      if (busy) return;
      busy = true; more.disabled = true;
      try {
        const params = new URLSearchParams({ view: "operations", requestId: request.id, limit: "25", includeHealthChecks: "true" });
        if (next) params.set("cursor", next);
        const result = await api(`/api/access?${params}`);
        for (const event of result.events) {
          const item = element("div");
          item.append(element("div", "", `${formatTime(event.occurredAt)} · ${operationLabel(event.operation, t)} · ${t(({allowed:"Allowed",denied:"Denied",failed:"Failed"})[event.decision] || event.decision)} · ${event.telemetry?.durationMs ?? "—"} ms · ${t("Input")}: ${event.telemetry?.inputCount ?? "—"}`),
            technicalDetails(element, t("Identifiers and implementation"), [
              [t("Operation identifier"), event.operation], [t("Occurred"), event.occurredAt],
              [t("Read projections"), event.telemetry?.projections?.join(", ")],
            ]));
          list.append(item);
        }
        if (!started && !result.events.length) list.append(element("div", "muted", t("No retained internal operations")));
        started = true; next = result.nextCursor; more.hidden = !next;
      } catch (error) { showError(error); } finally { busy = false; more.disabled = false; }
    }
    more.onclick = () => void loadChildren();
    return () => { if (!started) void loadChildren(); };
  }
  function render({ appendFrom = 0 } = {}) {
    if (!current) return;
    const clients = byId("access-clients");
    const all = element("button", "access-client", t("All clients"));
    all.type = "button"; all.setAttribute("aria-pressed", String(!principalId));
    all.onclick = () => selectClient("");
    all.append(element("span", "", `${formatNumber(current.clients.reduce((sum, client) => sum + client.eventCount, 0))} ${unit()}`));
    clients.replaceChildren(all, ...current.clients.map(client => {
      const id = client.principal.principalId;
      clientNames.set(id, client.principal.displayName || id);
      const button = element("button", "access-client"); button.type = "button";
      button.setAttribute("aria-pressed", String(principalId === id));
      button.append(element("strong", "", client.principal.displayName || id));
      button.title = id;
      button.append(element("span", "", `${formatNumber(client.eventCount)} ${unit()}`),
        element("span", "muted", `${t("Last access")}: ${formatTime(client.lastAccessAt)}`));
      button.onclick = () => selectClient(id); return button;
    }));
    byId("access-selected").textContent = clientNames.get(principalId) || (principalId ? t("Selected client") : t("All clients"));
    const selectedOperation = byId("access-operation").value;
    current.operations.forEach(item => knownOperations.add(item.operation));
    if (selectedOperation) knownOperations.add(selectedOperation);
    const choices = [element("option", "", t("All operations")), ...[...knownOperations].sort((a, b) => operationLabel(a, t).localeCompare(operationLabel(b, t))).map(value => {
      const option = element("option", "", operationLabel(value, t)); option.value = value; return option;
    })];
    choices[0].value = "";
    byId("access-operation").replaceChildren(...choices); byId("access-operation").value = selectedOperation;
    byId("access-reset").hidden = !principalId && !since && !selectedOperation && !byId("access-health").checked && (byId("access-view").value || "requests") === "requests";
    const operationButtons = current.operations.map(item => {
      const button = element("button", "compact-button secondary-button", `${operationLabel(item.operation, t)} · ${formatNumber(item.eventCount)}`);
      button.setAttribute("aria-pressed", String(selectedOperation === item.operation));
      button.type = "button"; button.onclick = () => { byId("access-operation").value = item.operation; void load().catch(showError); }; return button;
    });
    const summary = byId("access-summary"); summary.replaceChildren(...operationButtons.slice(0, 5));
    if (operationButtons.length > 5) {
      const more = element("details", "access-more-operations"); more.open = moreOperationsOpen;
      more.append(element("summary", "", t("More operations ({count})").replace("{count}", formatNumber(operationButtons.length - 5))));
      const remaining = element("div", "access-operations"); remaining.append(...operationButtons.slice(5)); more.append(remaining);
      more.addEventListener("toggle", () => { moreOperationsOpen = more.open; }); summary.append(more);
    }
    const rows = events.slice(appendFrom).flatMap(event => {
      const row = document.createElement("tr");
      const client = element("td", "");
      client.append(element("div", "", event.principal.displayName || event.principal.principalId));

      const details = element("details", "access-event-details");
      const expandedRow = element("tr", "access-expanded-row"); expandedRow.hidden = true;
      const panel = element("td", "access-expanded-panel"); panel.colSpan = 5;
      expandedRow.append(panel);
      const summary = element("summary", "", event.telemetry ? `${t("Details")} · ${event.telemetry.durationMs} ms` : t("Details"));
      panel.id = `access-detail-${event.eventId}`; summary.setAttribute("aria-controls", panel.id);
      details.append(summary);
      panel.append(scopeDetails(event));
      panel.append(technicalDetails(element, t("Identifiers and implementation"), [
        [t("Client"), event.principal.principalId], [t("Session"), event.sessionId],
        [t("Operation identifier"), event.operation], [t("Scope identifier"), event.scopes.join(", ")],
        [t("Occurred"), event.occurredAt],
      ]));
      if (event.detail) panel.append(element("p", "", event.detail));
      let expand = () => {};
      if (event.telemetry) {
        const v = event.telemetry;
        if (v.request?.root) expand = attachOperations(panel, v.request);
        else panel.append(element("div", "muted", `${v.durationMs} ms · ${t("Input")}: ${v.inputCount ?? "—"} · ${t("Output")}: ${v.outputCount ?? "—"}`));
        if (v.projections?.length) panel.append(element("div", "mono", v.projections.join(", ")));
      }
      details.addEventListener("toggle", () => { expandedRow.hidden = !details.open; if (details.open) expand(); });
      const detailCell = element("td", ""); detailCell.append(details);
      row.append(element("td", "", formatTime(event.occurredAt)), client, element("td", "", operationLabel(event.operation, t)),
        element("td", event.decision === "allowed" ? "decision-allowed" : "decision-denied", t(({allowed:"Allowed",denied:"Denied",failed:"Failed"})[event.decision] || event.decision)), detailCell);
      return [row, expandedRow];
    });
    if (!rows.length) { const row = document.createElement("tr"), cell = element("td", "empty", t("No matching access events")); cell.colSpan = 5; row.append(cell); rows.push(row); }
    if (appendFrom) byId("access-rows").append(...rows);
    else byId("access-rows").replaceChildren(...rows);
    byId("access-loaded").textContent = `${formatNumber(events.length)} / ${formatNumber(current.totalEvents)} ${unit()}`;
    byId("access-more").hidden = !cursor;
  }
  async function load({ append = false } = {}) {
    if (append && !cursor) return;
    const request = ++generation;
    const params = filters(); if (append) params.set("cursor", cursor);
    byId("access-more").disabled = true;
    byId("access-status").textContent = t("Loading");
    byId("access-rows").setAttribute("aria-busy", "true");
    try {
      const result = await api(`/api/access?${params}`);
      if (request !== generation) return;
      scopeNames = append ? { ...scopeNames, ...result.scopeNames } : result.scopeNames || {};
      const appendFrom = append ? events.length : 0;
      current = result; cursor = result.nextCursor; events = append ? events.concat(result.events) : result.events;
      loaded = true; render({ appendFrom }); byId("access-status").textContent = t("Audit timeline");
    } catch (error) {
      if (request !== generation) return;
      byId("access-status").textContent = t("Load failed"); throw error;
    } finally {
      if (request === generation) { byId("access-more").disabled = false; byId("access-rows").setAttribute("aria-busy", "false"); }
    }
  }
  function selectClient(id) { principalId = id; void load().catch(showError); }
  byId("access-filters").addEventListener("submit", event => { event.preventDefault(); void load().catch(showError); });
  byId("access-period").addEventListener("change", () => {
    const hours = Number(byId("access-period").value); since = hours ? new Date(Date.now() - hours * 3600000).toISOString() : "";
    void load().catch(showError);
  });
  byId("access-view").addEventListener("change", () => { knownOperations.clear(); byId("access-operation").value = ""; void load().catch(showError); });
  byId("access-operation").addEventListener("change", () => { void load().catch(showError); });
  byId("access-reset").addEventListener("click", () => {
    principalId = ""; since = ""; knownOperations.clear(); moreOperationsOpen = false;
    byId("access-period").value = "0"; byId("access-view").value = "requests";
    byId("access-operation").value = ""; byId("access-health").checked = false;
    void load().catch(showError);
  });
  byId("access-health").addEventListener("change", () => { void load().catch(showError); });
  byId("access-more").addEventListener("click", () => { void load({ append: true }).catch(showError); });
  return { load, render, get loaded() { return loaded; } };
}
