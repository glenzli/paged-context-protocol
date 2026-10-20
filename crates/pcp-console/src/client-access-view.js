import { technicalDetails, scopeLabel } from "./console-presentation.js";

const ACCESS_MODES = {
  observe: "Observe service", read: "Read memories", audit: "Inspect access records",
  contribute: "Contribute memories", write: "Write and revise memories",
  repair: "Repair memories", admin: "Full administration",
};

// Owns enrollment snapshots, permission disclosure and approval/revocation actions.
export function createClientAccessView({ api, mutate, byId, element, t, scopeName, formatTime, showError, confirmAction }) {
  let snapshot = null, signature = "", generation = 0, available = false, busy = false;
  const seenPending = new Set();
  const roots = () => [byId("enrollment-pending"), byId("enrollment-registered")];
  const name = scope => scope === "user:self" ? t("Personal memory") : scopeName(scope);

  function permissionDetails(item, pending, id) {
    const access = pending ? item.requested_access : item.approved_access;
    const details = element("details", "client-permissions"); details.dataset.clientDisclosure = id;
    const summary = element("summary", "", t(pending ? "Requested permissions" : "Permissions and scopes"));
    summary.dataset.clientFocus = `permissions:${id}`;
    details.append(summary);
    const fields = element("dl", "details-grid client-permission-fields");
    const field = (label, value) => fields.append(element("dt", "", t(label)), typeof value === "object" ? value : element("dd", "", value));
    field("Access mode", t(ACCESS_MODES[access.mode] || access.mode));
    field("Scope policy", access.read_all_scopes ? t("Read all scopes; other permissions remain scoped") : t("Permissions apply to listed scopes"));
    const fixed = element("dd"); fixed.append(scopeLabel(element, access.scopes || [], name, "span", "", {"user:self": t("Personal memory")}));
    field("Configured scopes", access.scopes?.length ? fixed : t("None"));
    if (item.project_scopes?.length) {
      const projects = element("dd"), list = element("ul", "client-project-list");
      for (const scope of item.project_scopes) { const row = element("li"); row.append(scopeLabel(element, [scope], name)); list.append(row); }
      projects.append(list); field("Writable projects", projects);
    }
    field("Cross-scope derivation", t(access.allow_cross_scope_derivation ? "Allowed" : "Not allowed"));
    if (!pending) field("Register new projects", t(item.allow_project_registration ? "Allowed" : "Not allowed"));
    details.append(fields);
    return details;
  }

  function setBusy(value) {
    busy = value;
    byId("enrollment-dialog").setAttribute("aria-busy", String(value));
    for (const root of roots()) for (const button of root.querySelectorAll("button")) button.disabled = value;
  }

  function action(label, style, id, path, body, confirmation) {
    const button = element("button", style, t(label)); button.type = "button";
    button.disabled = busy; button.dataset.clientFocus = `${label}:${id}`;
    button.addEventListener("click", async () => {
      if (busy) return;
      if (confirmation && !await confirmAction(confirmation)) return;
      // Invalidate a poll that started before this operator decision.
      generation++; setBusy(true);
      try { await mutate(path, body); await load({ autoOpen: false, force: true }); }
      catch (error) { showError(error); }
      finally { setBusy(false); }
    });
    return button;
  }

  function row(item, pending) {
    const id = pending ? item.request_id : item.registration_id;
    const principal = item.client.principal, access = pending ? item.requested_access : item.approved_access;
    const clientName = principal.displayName || principal.principalId;
    const row = element("article", "enrollment-row");
    const identity = element("div", "enrollment-identity");
    identity.append(element("strong", "", clientName), element("span", "muted", t(ACCESS_MODES[access.mode] || access.mode)));
    const summary = access.read_all_scopes ? t("Reads all scopes") : t("{count} configured scopes").replace("{count}", access.scopes?.length || 0);
    identity.append(element("span", "muted", `${summary}${item.project_scopes?.length ? ` · ${t("{count} writable projects").replace("{count}", item.project_scopes.length)}` : ""}`));
    const actions = element("div", "enrollment-actions");
    if (pending) {
      const path = `/api/enrollment/requests/${encodeURIComponent(id)}`;
      actions.append(action("Reject", "secondary-button", id, `${path}/reject`), action("Approve", "primary-button", id, `${path}/approve`));
    } else {
      const path = `/api/enrollment/registrations/${encodeURIComponent(id)}`;
      if (["contribute", "write", "admin"].includes(access.mode)) {
        const enabled = item.allow_project_registration === true;
        actions.append(action(enabled ? "Disable project registration" : "Enable project registration", "secondary-button", id, `${path}/project-policy`, { enabled: !enabled }));
      }
      actions.append(action("Revoke", "danger-button", id, `${path}/revoke`, undefined, {
        title: t("Revoke client access?"), description: t("This client will lose its approved connection. Stored memories are retained."), confirmLabel: t("Revoke"),
      }));
    }
    const permissions = permissionDetails(item, pending, id);
    // A pending approval must expose the requested authority before its action.
    permissions.open = pending;
    permissions.append(actions);
    const metadata = technicalDetails(element, t("Connection and identifiers"), [
      [t("Client"), principal.principalId], [t(pending ? "Request ID" : "Registration ID"), id],
      [t(pending ? "Requested at" : "Registered at"), formatTime(pending ? item.requested_at : item.created_at)],
      [t(pending ? "Expires at" : "Last connected"), pending ? formatTime(item.expires_at) : item.last_opened_at ? formatTime(item.last_opened_at) : t("Not connected yet")],
    ]);
    metadata.dataset.clientDisclosure = `metadata:${id}`;
    metadata.children[0].dataset.clientFocus = `metadata:${id}`;
    const body = element("div", "client-access-details"); body.append(permissions, metadata);
    row.append(identity, body);
    return row;
  }

  function render(force = false) {
    if (!snapshot) return;
    const next = JSON.stringify([snapshot, t("Client access")]);
    if (!force && signature === next) return;
    const open = new Map(roots().flatMap(root => [...root.querySelectorAll("details[data-client-disclosure]")].map(detail => [detail.dataset.clientDisclosure, detail.open])));
    const focused = roots().some(root => root.contains(document.activeElement)) ? document.activeElement?.dataset.clientFocus : null;
    const pending = snapshot.pending || [], registrations = snapshot.registrations || [];
    for (const [root, items, isPending, empty] of [[byId("enrollment-pending"), pending, true, "No pending requests"], [byId("enrollment-registered"), registrations, false, "No approved clients"]]) {
      root.replaceChildren(...items.map(item => row(item, isPending)));
      if (!items.length) root.append(element("div", "empty enrollment-empty", t(empty)));
      for (const detail of root.querySelectorAll("details[data-client-disclosure]")) if (open.has(detail.dataset.clientDisclosure)) detail.open = open.get(detail.dataset.clientDisclosure);
      if (focused) for (const target of root.querySelectorAll("[data-client-focus]")) if (target.dataset.clientFocus === focused) target.focus({preventScroll: true});
    }
    signature = next;
  }

  async function load({ autoOpen = true, force = false } = {}) {
    if (busy && !force) return;
    const request = ++generation;
    try {
      const data = await api("/api/enrollment");
      if (request !== generation) return;
      if (data.result.status !== "snapshot") throw new Error("Unexpected enrollment response");
      snapshot = data.result; available = true;
      byId("enrollment-open").hidden = false;
      const pending = snapshot.pending || [], badge = byId("enrollment-badge");
      badge.textContent = String(pending.length); badge.hidden = !pending.length;
      byId("enrollment-pending-section").hidden = !pending.length;
      render(force);
      if (autoOpen && pending.some(item => !seenPending.has(item.request_id)) && !document.querySelector("dialog[open]")) byId("enrollment-dialog").showModal();
      if (byId("enrollment-dialog").open) pending.forEach(item => seenPending.add(item.request_id));
    } catch (error) {
      if (request !== generation) return;
      if (!available) byId("enrollment-open").hidden = true;
      if (!autoOpen) showError(error);
    }
  }

  function open() { byId("enrollment-dialog").showModal(); return load({ autoOpen: false }); }
  return { load, open, rerender: () => render(true) };
}
