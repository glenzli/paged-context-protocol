// The audit view owns its filters, request generation, pagination and rendering.
export function createAccessView({ api, byId, element, t, formatTime, formatNumber, showError }) {
  let loaded = false, generation = 0, cursor = null, events = [], current = null;
  let principalId = "", since = "";
  function filters() {
    const params = new URLSearchParams({ limit: "50", view: byId("access-view").value || "requests", includeHealthChecks: String(byId("access-health").checked) });
    if (principalId) params.set("principalId", principalId);
    if (since) params.set("since", since);
    const operation = byId("access-operation").value.trim();
    if (operation) params.set("operation", operation);
    return params;
  }
  function unit() { return t((byId("access-view").value || "requests") === "requests" ? "requests" : "events"); }
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
        result.events.forEach(event => list.append(element("div", "mono", `${event.occurredAt} · ${event.operation} · ${event.decision} · ${event.telemetry?.durationMs ?? "—"} ms · ${t("Input")}: ${event.telemetry?.inputCount ?? "—"} · ${event.telemetry?.projections?.join(", ") || ""}`)));
        if (!started && !result.events.length) list.append(element("div", "muted", t("No retained internal operations")));
        started = true; next = result.nextCursor; more.hidden = !next;
      } catch (error) { showError(error); } finally { busy = false; more.disabled = false; }
    }
    more.onclick = () => void loadChildren();
    return () => { if (!started) void loadChildren(); };
  }
  function render() {
    if (!current) return;
    const clients = byId("access-clients");
    const all = element("button", "access-client", t("All clients"));
    all.type = "button"; all.setAttribute("aria-pressed", String(!principalId));
    all.onclick = () => selectClient("");
    clients.replaceChildren(all, ...current.clients.map(client => {
      const id = client.principal.principalId;
      const button = element("button", "access-client"); button.type = "button";
      button.setAttribute("aria-pressed", String(principalId === id));
      button.append(element("strong", "", client.principal.displayName || id));
      if (client.principal.displayName && client.principal.displayName !== id) button.append(element("span", "mono muted", id));
      button.append(element("span", "", `${formatNumber(client.eventCount)} ${unit()}`),
        element("span", "muted", `${t("Last access")}: ${formatTime(client.lastAccessAt)}`));
      button.onclick = () => selectClient(id); return button;
    }));
    byId("access-selected").textContent = principalId || t("All clients");
    byId("access-summary").replaceChildren(...current.operations.map(item => {
      const button = element("button", "compact-button secondary-button", `${item.operation} · ${formatNumber(item.eventCount)}`);
      button.type = "button"; button.onclick = () => { byId("access-operation").value = item.operation; void load().catch(showError); }; return button;
    }));
    const rows = events.flatMap(event => {
      const row = document.createElement("tr");
      const client = element("td", "");
      client.append(element("div", "", event.principal.displayName || event.principal.principalId));
      if (event.principal.displayName) client.append(element("div", "mono muted", event.principal.principalId));
      const details = element("details", "access-event-details");
      const expandedRow = element("tr", "access-expanded-row"); expandedRow.hidden = true;
      const panel = element("td", "access-expanded-panel"); panel.colSpan = 6;
      expandedRow.append(panel);
      const summary = element("summary", "", event.telemetry?.request?.root ? `${event.telemetry.durationMs} ms · ${event.telemetry.request.batchReads + event.telemetry.request.singleReads} ${t("reads")}` : t("Details"));
      panel.id = `access-detail-${event.eventId}`; summary.setAttribute("aria-controls", panel.id);
      details.append(summary);
      panel.append(element("div", "mono muted", `${t("Session")}: ${event.sessionId}`));
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
      row.append(element("td", "", `${formatTime(event.occurredAt)}.${event.occurredAt.match(/\.(\d{3})/)?.[1] || "000"}`), client, element("td", "mono", event.operation),
        element("td", "mono", event.scopes.join(", ")), element("td", event.decision === "allowed" ? "decision-allowed" : "decision-denied", event.decision), detailCell);
      return [row, expandedRow];
    });
    if (!rows.length) { const row = document.createElement("tr"), cell = element("td", "empty", t("No matching access events")); cell.colSpan = 6; row.append(cell); rows.push(row); }
    byId("access-rows").replaceChildren(...rows);
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
      current = result; cursor = result.nextCursor; events = append ? events.concat(result.events) : result.events;
      loaded = true; render(); byId("access-status").textContent = t("Audit timeline");
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
  byId("access-view").addEventListener("change", () => { byId("access-operation").value = ""; void load().catch(showError); });
  byId("access-health").addEventListener("change", () => { void load().catch(showError); });
  byId("access-more").addEventListener("click", () => { void load({ append: true }).catch(showError); });
  return { load, render, get loaded() { return loaded; } };
}
