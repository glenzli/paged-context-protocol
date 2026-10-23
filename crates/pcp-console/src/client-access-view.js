import { technicalDetails, scopeLabel } from "./console-presentation.js";

const ACCESS_MODES = {
  observe: "Observe service", read: "Read memories", audit: "Inspect access records",
  contribute: "Contribute memories", write: "Write and revise memories",
  repair: "Repair memories", admin: "Full administration",
};

// Owns enrollment snapshots, permission disclosure and approval/revocation actions.
export function createClientAccessView({ api, mutate, byId, element, t, scopeName, formatTime, confirmAction, localSetup }) {
  let snapshot = null, signature = "", generation = 0, available = false, busy = false, checking = false, uncertain = false, refreshRequired = false;
  let feedback = null;
  const seenPending = new Set();
  const roots = () => [byId("enrollment-pending"), byId("enrollment-registered"), ...(localSetup ? [byId("enrollment-local")] : [])];
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
    field("Scope policy", access.all_scopes ? t("Includes every current and future scope. Access follows the selected mode.")
      : access.read_all_scopes ? t("Read all scopes; other permissions remain scoped") : t("Permissions apply to listed scopes"));
    const fixed = element("dd"); fixed.append(scopeLabel(element, access.scopes || [], name, "span", "", {"user:self": t("Personal memory")}));
    field("Scope access", access.all_scopes ? t("All scopes") : access.scopes?.length ? fixed : t("None"));
    if (!access.all_scopes && item.project_scopes?.length) {
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
    const localMutating = localSetup?.isMutating() || false;
    byId("enrollment-dialog").setAttribute("aria-busy", String(value || checking || localMutating));
    for (const root of roots()) for (const button of root.querySelectorAll("button")) button.disabled = button.dataset.localSetupProfile
      ? value || checking || localMutating || !localSetup.canOpenProfile(button.dataset.localSetupProfile)
      : Boolean(value || checking || uncertain || refreshRequired || localSetup?.blocksMutation());
    byId("enrollment-refresh").disabled = value || checking || localMutating;
    byId("enrollment-close").disabled = value || localMutating;
    byId("client-context-permissions").disabled = value || localMutating;
    localSetup?.setExternalBusy(value || checking || uncertain || refreshRequired);
  }

  function renderFeedback() {
    const notice = byId("enrollment-error");
    notice.hidden = !feedback;
    byId("enrollment-error-message").textContent = feedback
      ? `${feedback.unknown ? `${t("The action outcome is unknown. Refresh permissions and check the current state before another action.")}\n`
        : refreshRequired ? `${t("The action completed, but permissions could not refresh. Refresh permissions before another action.")}\n` : ""}${feedback.error.message || String(feedback.error)}` : "";
  }

  function fail(error, unknown = false) {
    feedback = { error, unknown };
    renderFeedback();
    byId("enrollment-error").focus({ preventScroll: false });
  }

  function action(label, style, id, path, body, confirmation) {
    const button = element("button", style, t(label)); button.type = "button";
    button.disabled = busy || checking || uncertain || refreshRequired; button.dataset.clientFocus = `${label}:${id}`;
    button.addEventListener("click", async () => {
      if (busy || checking || uncertain || refreshRequired || localSetup?.blocksMutation()) return;
      if (confirmation && !await confirmAction(confirmation)) return;
      // Invalidate a poll that started before this operator decision.
      generation++; feedback = null; renderFeedback(); setBusy(true);
      try {
        await mutate(path, body);
        refreshRequired = true;
        await load({ autoOpen: false, force: true, reconcile: true });
      }
      catch (error) { uncertain = error.mutationOutcome !== "not_applied"; fail(error, uncertain); }
      finally { setBusy(false); }
    });
    return button;
  }

  function localAction(profile) {
    const button = element("button", "secondary-button", t(profile.status === "pending" ? "Review permissions" : profile.status === "ready" ? "MCP configuration" : "View setup"));
    button.type = "button"; button.dataset.clientFocus = `setup:${profile.id}`;
    button.dataset.localSetupProfile = profile.id;
    button.addEventListener("click", () => localSetup.openProfile(profile.id));
    return button;
  }

  function row(item, pending, local) {
    const id = pending ? item.request_id : item.registration_id;
    const principal = item.client.principal, access = pending ? item.requested_access : item.approved_access;
    const clientName = principal.displayName || principal.principalId;
    const row = element("article", "enrollment-row");
    const identity = element("div", "enrollment-identity");
    identity.append(element("strong", "", clientName), element("span", "muted", t(ACCESS_MODES[access.mode] || access.mode)));
    const summary = access.all_scopes ? t("All scopes") : access.read_all_scopes ? t("Reads all scopes") : t("{count} configured scopes").replace("{count}", access.scopes?.length || 0);
    identity.append(element("span", "muted", `${summary}${!access.all_scopes && item.project_scopes?.length ? ` · ${t("{count} writable projects").replace("{count}", item.project_scopes.length)}` : ""}`));
    if (local) identity.append(element("span", "local-client-status", localSetup.statusLabel(local)));
    const actions = element("div", "enrollment-actions");
    if (pending) {
      const path = `/api/enrollment/requests/${encodeURIComponent(id)}`;
      actions.append(action("Reject", "secondary-button", id, `${path}/reject`), local ? localAction(local) : action("Approve", "primary-button", id, `${path}/approve`));
    } else {
      const path = `/api/enrollment/registrations/${encodeURIComponent(id)}`;
      if (["contribute", "write", "admin"].includes(access.mode)) {
        const enabled = item.allow_project_registration === true;
        actions.append(action(enabled ? "Disable project registration" : "Enable project registration", "secondary-button", id, `${path}/project-policy`, { enabled: !enabled }));
      }
      actions.append(action("Revoke", "danger-button", id, `${path}/revoke`, undefined, {
        title: t("Revoke client access?"), description: t("This client will lose its approved connection. Stored memories are retained."), confirmLabel: t("Revoke"),
      }));
      if (local) actions.append(localAction(local));
    }
    const permissions = permissionDetails(item, pending, id);
    // A pending approval must expose the requested authority before its action.
    permissions.open = pending;
    permissions.append(actions);
    const metadata = technicalDetails(element, t("Connection and identifiers"), [
      [t("Client"), principal.principalId], [t(pending ? "Request ID" : "Registration ID"), id],
      [t(pending ? "Requested at" : "Registered at"), formatTime(pending ? item.requested_at : item.created_at)],
      [t(pending ? "Expires at" : local ? "Runtime session last opened" : "Last connected"), pending ? formatTime(item.expires_at) : item.last_opened_at ? formatTime(item.last_opened_at) : t(local ? "Connection not verified" : "Not connected yet")],
    ]);
    metadata.dataset.clientDisclosure = `metadata:${id}`;
    metadata.children[0].dataset.clientFocus = `metadata:${id}`;
    const body = element("div", "client-access-details"); body.append(permissions, metadata);
    row.append(identity, body);
    return row;
  }

  function localRow(profile) {
    const pending = snapshot.pending?.find(value => value.request_id === profile.requestId);
    const registered = snapshot.registrations?.find(value => value.registration_id === profile.registrationId);
    if (pending) return row(pending, true, profile);
    if (registered) return row(registered, false, profile);
    const result = element("article", "enrollment-row"), identity = element("div", "enrollment-identity");
    identity.append(element("strong", "", profile.displayName), element("span", "muted", t(ACCESS_MODES[profile.mode] || profile.mode)),
      element("span", "local-client-status", localSetup.statusLabel(profile)));
    const scopes = element("div", "local-client-scope-summary");
    scopes.append(profile.allScopes ? element("span", "", t("All scopes")) : scopeLabel(element, profile.scopes || [], name, "span", "", {"user:self": t("Personal memory")}));
    identity.append(scopes);
    if (profile.reason) identity.append(element("span", "muted", profile.reason));
    const actions = element("div", "enrollment-actions"); actions.append(localAction(profile));
    result.append(identity, actions); return result;
  }

  function render(force = false) {
    if (!snapshot) return;
    const localClients = localSetup?.snapshot()?.clients || [];
    const next = JSON.stringify([snapshot, localClients, t("Client access")]);
    if (!force && signature === next) return;
    const open = new Map(roots().flatMap(root => [...root.querySelectorAll("details[data-client-disclosure]")].map(detail => [detail.dataset.clientDisclosure, detail.open])));
    const focused = roots().some(root => root.contains(document.activeElement)) ? document.activeElement?.dataset.clientFocus : null;
    const pending = (snapshot.pending || []).filter(item => !localClients.some(local => local.requestId === item.request_id));
    const registrations = (snapshot.registrations || []).filter(item => !localClients.some(local => local.registrationId === item.registration_id));
    const registeredTitle = localClients.length ? "Other approved clients" : "Approved clients";
    byId("enrollment-registered-title").dataset.i18n = registeredTitle;
    byId("enrollment-registered-title").textContent = t(registeredTitle);
    for (const [root, items, isPending, empty] of [[byId("enrollment-pending"), pending, true, "No pending requests"], [byId("enrollment-registered"), registrations, false, localClients.length ? "No other approved clients" : "No approved clients"]]) {
      root.replaceChildren(...items.map(item => row(item, isPending)));
      if (!items.length) root.append(element("div", "empty enrollment-empty", t(empty)));
      for (const detail of root.querySelectorAll("details[data-client-disclosure]")) if (open.has(detail.dataset.clientDisclosure)) detail.open = open.get(detail.dataset.clientDisclosure);
      if (focused) for (const target of root.querySelectorAll("[data-client-focus]")) if (target.dataset.clientFocus === focused) target.focus({preventScroll: true});
    }
    if (localSetup) {
      const root = byId("enrollment-local"); root.replaceChildren(...localClients.map(localRow));
      byId("enrollment-local-section").hidden = !localClients.length;
      for (const detail of root.querySelectorAll("details[data-client-disclosure]")) if (open.has(detail.dataset.clientDisclosure)) detail.open = open.get(detail.dataset.clientDisclosure);
      if (focused) for (const target of root.querySelectorAll("[data-client-focus]")) if (target.dataset.clientFocus === focused) target.focus({preventScroll: true});
    }
    byId("enrollment-pending-section").hidden = !pending.length;
    signature = next;
  }

  async function load({ autoOpen = true, force = false, reconcile = false } = {}) {
    if ((busy || checking || uncertain || refreshRequired || localSetup?.isMutating()) && !force) return;
    const request = ++generation;
    try {
      const data = await api("/api/enrollment");
      if (request !== generation) return;
      if (data.result.status !== "snapshot") throw new Error("Unexpected enrollment response");
      snapshot = data.result; available = true;
      await localSetup?.load();
      if (request !== generation) return;
      if (reconcile) { uncertain = false; refreshRequired = false; feedback = null; renderFeedback(); }
      byId("enrollment-open").hidden = false;
      const pending = snapshot.pending || [], badge = byId("enrollment-badge");
      badge.textContent = String(pending.length); badge.hidden = !pending.length;
      render(force);
      if (autoOpen && pending.some(item => !seenPending.has(item.request_id)) && !document.querySelector("dialog[open]")) byId("enrollment-dialog").showModal();
      if (byId("enrollment-dialog").open) pending.forEach(item => seenPending.add(item.request_id));
      setBusy(busy);
    } catch (error) {
      if (request !== generation) return;
      if (!available) byId("enrollment-open").hidden = true;
      if (!autoOpen) fail(error, uncertain);
    }
  }

  byId("enrollment-refresh").addEventListener("click", async () => {
    if (busy || checking) return;
    checking = true; setBusy(busy);
    try { await load({ autoOpen: false, force: true, reconcile: true }); }
    finally { checking = false; setBusy(busy); }
  });
  byId("enrollment-dialog").addEventListener("cancel", event => { if (busy || localSetup?.isMutating()) event.preventDefault(); });
  function open() { byId("enrollment-dialog").showModal(); renderFeedback(); return load({ autoOpen: false }); }
  return { load, open, rerender: () => { render(true); renderFeedback(); setBusy(busy); }, hasPendingAction: () => Boolean(busy || uncertain || localSetup?.hasPendingAction()) };
}
