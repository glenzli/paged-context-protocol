// The audit view owns its filters, request generation, pagination and rendering.
export function createAccessView({ api, byId, element, t, formatTime, formatNumber, showError }) {
  let loaded = false, generation = 0, cursor = null, events = [], current = null;
  let principalId = "", since = "";
  function filters() {
    const params = new URLSearchParams({ limit: "50", includeHealthChecks: String(byId("access-health").checked) });
    if (principalId) params.set("principalId", principalId);
    if (since) params.set("since", since);
    const operation = byId("access-operation").value.trim();
    if (operation) params.set("operation", operation);
    return params;
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
      button.append(element("span", "", `${formatNumber(client.eventCount)} ${t("events")}`),
        element("span", "muted", `${t("Last access")}: ${formatTime(client.lastAccessAt)}`));
      button.onclick = () => selectClient(id); return button;
    }));
    byId("access-selected").textContent = principalId || t("All clients");
    byId("access-summary").replaceChildren(...current.operations.map(item => {
      const button = element("button", "compact-button secondary-button", `${item.operation} · ${formatNumber(item.eventCount)}`);
      button.type = "button"; button.onclick = () => { byId("access-operation").value = item.operation; void load().catch(showError); }; return button;
    }));
    const rows = events.map(event => {
      const row = document.createElement("tr");
      const client = element("td", "");
      client.append(element("div", "", event.principal.displayName || event.principal.principalId));
      if (event.principal.displayName) client.append(element("div", "mono muted", event.principal.principalId));
      const details = element("details", "access-event-details");
      details.append(element("summary", "", t("Details")), element("div", "mono", `${t("Session")}: ${event.sessionId}`));
      if (event.detail) details.append(element("p", "", event.detail));
      if (event.telemetry) {
        const v = event.telemetry;
        details.append(element("div", "muted", `${v.durationMs} ms · ${t("Input")}: ${v.inputCount ?? "—"} · ${t("Output")}: ${v.outputCount ?? "—"}`));
        if (v.projections?.length) details.append(element("div", "mono", v.projections.join(", ")));
      }
      const detailCell = element("td", ""); detailCell.append(details);
      row.append(element("td", "", formatTime(event.occurredAt)), client, element("td", "mono", event.operation),
        element("td", "mono", event.scopes.join(", ")), element("td", event.decision === "allowed" ? "decision-allowed" : "decision-denied", event.decision), detailCell);
      return row;
    });
    if (!rows.length) { const row = document.createElement("tr"), cell = element("td", "empty", t("No matching access events")); cell.colSpan = 6; row.append(cell); rows.push(row); }
    byId("access-rows").replaceChildren(...rows);
    byId("access-loaded").textContent = `${formatNumber(events.length)} / ${formatNumber(current.totalEvents)} ${t("events")}`;
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
  byId("access-health").addEventListener("change", () => { void load().catch(showError); });
  byId("access-more").addEventListener("click", () => { void load({ append: true }).catch(showError); });
  return { load, render, get loaded() { return loaded; } };
}
