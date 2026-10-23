import { LOCAL_CLIENT_TARGETS, localClientTarget, renderLocalClientConfiguration, localClientVerificationPrompt, localClientUsageGuidance } from "./local-client-config.js";

const MODES = { read: "Read memories", contribute: "Read and contribute" };
const STATUSES = {
  pending: "Awaiting approval", ready: "Ready to connect", revoked: "Access revoked",
  rejected: "Registration rejected", unavailable: "State unavailable",
};

function saveConfiguration(value, filename, mimeType = "application/json") {
  const url = URL.createObjectURL(new Blob([value], { type: mimeType }));
  const link = document.createElement("a"); link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove(); URL.revokeObjectURL(url);
}

const clientSelection = {
  get(id) { try { return localStorage.getItem(`pcp-console.local-client-target.${id}`); } catch { return null; } },
  set(id, value) { try { localStorage.setItem(`pcp-console.local-client-target.${id}`, value); } catch { /* Optional browser preference. */ } },
};

// Owns the local profile inventory, unsubmitted draft, explicit grant and read-only recovery.
export function createLocalClientSetup({ api, mutate, byId, element, t, scopeName, confirmAction,
  changed = () => {}, committed = async () => {}, operationId = () => crypto.randomUUID(),
  copy = value => navigator.clipboard.writeText(value), download = saveConfiguration, selection = clientSelection }) {
  let inventory = null, inventoryGeneration = 0, generation = 0;
  const newDraft = () => ({ displayName: "", mode: "read", allScopes: true, scopes: [] });
  let draft = newDraft(), profile = null, operation = null, config = null;
  let targetId = "generic";
  let stage = "edit", busy = false, loading = false, uncertain = false, resumable = false, externalBusy = false, feedback = null, notice = "", scopeQuery = "";
  const dialog = byId("local-client-dialog");
  const targetSelect = byId("local-client-target");
  targetSelect.replaceChildren(...LOCAL_CLIENT_TARGETS.map(target => {
    const option = element("option", "", t(target.name)); option.value = target.id; return option;
  }));
  const exportValue = () => renderLocalClientConfiguration(config, targetId);
  const name = namespace => namespace === "user:self" ? t("Personal memory")
    : inventory?.scopeOptions?.find(item => item.namespace === namespace)?.displayName || scopeName(namespace);
  const path = id => `/api/local-clients/${encodeURIComponent(id)}`;
  const descriptor = value => ({ displayName: value.displayName, mode: value.mode, allScopes: value.allScopes === true, scopes: value.allScopes ? [] : [...value.scopes].sort() });
  const sameGrant = (left, right) => JSON.stringify(descriptor(left)) === JSON.stringify(descriptor(right));
  const dirty = () => stage === "edit" && (draft.displayName.trim() !== "" || draft.mode !== "read" || !draft.allScopes);
  const editable = () => stage === "edit" && !busy && !loading && !uncertain && !externalBusy;
  const statusLabel = value => t(STATUSES[value.status] || "State unavailable");

  function validateProfile(value, expectedId) {
    if (!value || value.id !== expectedId || !MODES[value.mode] || !Array.isArray(value.scopes)
        || (value.allScopes !== undefined && typeof value.allScopes !== "boolean")
        || (value.allScopes === true && value.scopes.length !== 0)
        || !Object.hasOwn(STATUSES, value.status)) throw new Error(t("Unexpected local agent response."));
    return value;
  }

  function error(error, unknown = false) {
    feedback = { error, unknown }; render();
    if (dialog.open) byId("local-client-error").focus({ preventScroll: false });
  }

  function renderScopeOptions() {
    const root = byId("local-client-scope-options"), focused = document.activeElement?.dataset.localScope;
    const options = (inventory?.scopeOptions || []).filter(scope => `${name(scope.namespace)} ${scope.namespace}`.toLocaleLowerCase().includes(scopeQuery.toLocaleLowerCase()));
    root.replaceChildren(...options.map(scope => {
      const label = element("label", "local-client-scope-option"), input = element("input");
      input.type = "checkbox"; input.value = scope.namespace; input.checked = draft.scopes.includes(scope.namespace);
      input.disabled = !editable(); input.dataset.localScope = scope.namespace;
      const caption = element("span"); caption.append(element("strong", "", name(scope.namespace)));
      input.addEventListener("change", () => {
        if (!editable()) return;
        draft.scopes = input.checked ? [...new Set([...draft.scopes, scope.namespace])] : draft.scopes.filter(value => value !== scope.namespace);
        feedback = null; render(false); changed();
      });
      label.append(input, caption); return label;
    }));
    if (!options.length && inventory?.scopeOptions?.length) root.append(element("p", "muted", t("No matching scopes")));
    if (focused) for (const node of root.querySelectorAll("input")) if (node.dataset.localScope === focused) node.focus({ preventScroll: true });
    byId("local-client-no-scopes").hidden = Boolean(inventory?.scopeOptions?.length);
  }

  function render(renderScopes = true) {
    const value = profile || operation;
    const add = byId("local-client-add");
    add.disabled = (!uncertain && inventory?.available !== true) || busy || loading || externalBusy;
    add.textContent = t(uncertain ? "Check local setup" : "Add local agent");
    const availability = byId("local-client-availability");
    availability.hidden = inventory === null || inventory.available === true;
    availability.textContent = inventory?.available === false
      ? `${t("Local agent setup is unavailable.")}${inventory.reason ? ` ${inventory.reason}` : ""}` : "";
    byId("local-client-form").hidden = stage !== "edit";
    byId("local-client-profile").hidden = stage === "edit";
    byId("local-client-name").disabled = !editable();
    byId("local-client-mode").disabled = !editable();
    byId("local-client-scope-mode").disabled = !editable();
    byId("local-client-scope-mode").value = draft.allScopes ? "all" : "selected";
    byId("local-client-selected-scopes").hidden = draft.allScopes;
    byId("local-client-scope-search").disabled = !editable() || draft.allScopes;
    byId("local-client-scope-help").textContent = t(draft.allScopes
      ? "Includes every current and future scope. Access follows the selected mode."
      : "Choose each scope explicitly. Other scopes are not included.");
    byId("local-client-mode-description").textContent = t(draft.mode === "contribute"
      ? "Read memories and contribute to the selected scopes." : "Read memories in the selected scopes.");
    byId("local-client-draft-status").hidden = !dirty();
    byId("local-client-register").disabled = !editable() || inventory?.available !== true;
    if (renderScopes) renderScopeOptions();
    else for (const node of byId("local-client-scope-options").querySelectorAll("input")) node.disabled = !editable();
    byId("local-client-scope-count").textContent = t("{count} scopes selected").replace("{count}", draft.scopes.length);
    dialog.setAttribute("aria-busy", String(busy || loading));
    byId("local-client-close").disabled = busy;
    byId("local-client-error").hidden = !feedback;
    byId("local-client-error-message").textContent = feedback
      ? `${feedback.unknown ? `${t("The setup outcome is unknown. Check the current state before another action.")}\n` : ""}${feedback.error.message || String(feedback.error)}` : "";
    byId("local-client-notice").hidden = !notice && !loading;
    byId("local-client-notice").textContent = loading ? t("Checking local agent…") : t(notice);
    byId("local-client-check").hidden = stage === "edit";
    byId("local-client-check").disabled = busy || loading;
    const pending = profile?.status === "pending";
    byId("local-client-approve").hidden = !pending;
    byId("local-client-approve").disabled = busy || loading || uncertain || externalBusy;
    byId("local-client-resume").hidden = !resumable;
    byId("local-client-resume").disabled = busy || loading || externalBusy;
    byId("local-client-review-note").hidden = !pending;
    if (value?.scopes) {
      byId("local-client-profile-name").textContent = value.displayName;
      byId("local-client-profile-mode").textContent = t(MODES[value.mode]);
      const scopes = byId("local-client-profile-scopes");
      scopes.replaceChildren(...(value.allScopes ? [element("li", "", t("All scopes"))]
        : value.scopes.map(namespace => element("li", "", name(namespace)))));
      byId("local-client-profile-status").textContent = uncertain ? t("Outcome unknown") : profile ? statusLabel(profile) : t("Outcome unknown");
      byId("local-client-profile-reason").hidden = !profile?.reason;
      byId("local-client-profile-reason").textContent = profile?.reason || "";
    }
    const ready = profile?.status === "ready" && !uncertain;
    targetSelect.disabled = busy || loading || uncertain;
    targetSelect.value = targetId;
    for (const option of targetSelect.children) option.textContent = t(localClientTarget(option.value).name);
    const target = localClientTarget(targetId);
    byId("local-client-location").textContent = t(target.location);
    byId("local-client-steps").replaceChildren(...target.steps.map(step => element("li", "", t(step))));
    const guide = byId("local-client-docs"); guide.href = target.docs;
    byId("local-client-config-section").hidden = !ready;
    let exported = null, exportError = "";
    if (config) try { exported = exportValue(); } catch { exportError = t("Unexpected local agent response."); }
    byId("local-client-config-error").hidden = !exportError;
    byId("local-client-config-error").textContent = exportError;
    const texts = {
      "local-client-config": exported?.text || "",
      "local-client-install-command": exported?.installCommand || "",
      "local-client-verification": exported && profile ? localClientVerificationPrompt(exported.serverName, profile, t) : "",
      "local-client-guidance": profile ? localClientUsageGuidance(profile.mode, t) : "",
    };
    for (const [id, text] of Object.entries(texts)) if (byId(id).value !== text) byId(id).value = text;
    byId("local-client-command-section").hidden = !exported?.installCommand;
    byId("local-client-expected-identity").textContent = profile?.principalId || "";
    for (const id of ["local-client-copy", "local-client-download", "local-client-copy-command", "local-client-copy-verification", "local-client-copy-guidance"])
      byId(id).disabled = !ready || !exported || busy || loading;
    byId("local-client-copy-command").disabled ||= !exported?.installCommand;
  }

  async function load() {
    const request = ++inventoryGeneration;
    try {
      const value = await api("/api/local-clients");
      if (request !== inventoryGeneration) return;
      if (typeof value?.available !== "boolean" || !Array.isArray(value.clients) || !Array.isArray(value.scopeOptions))
        throw new Error(t("Unexpected local agent response."));
      inventory = value;
    } catch (failure) {
      if (request !== inventoryGeneration) return;
      inventory = { ...(inventory || { clients: [], scopeOptions: [] }), available: false, reason: failure.message || String(failure) };
    }
    render(); changed();
  }

  async function configuration(request) {
    if (profile?.status !== "ready") return;
    const value = await api(`${path(profile.id)}/config`);
    if (request !== generation) return;
    const current = validateProfile(value?.client, profile.id);
    if (current.status !== "ready" || !sameGrant(current, profile) || !value.config?.mcpServers)
      throw new Error(t("Unexpected local agent response."));
    profile = current; config = value.config;
  }

  async function check() {
    if (busy || loading || !(profile?.id || operation?.operationId)) return;
    const id = profile?.id || operation.operationId, request = ++generation;
    loading = true; resumable = false; feedback = null; notice = ""; render(); changed();
    let refreshed = false;
    try {
      const value = validateProfile(await api(path(id)), id);
      if (request !== generation) return;
      if (operation?.scopes && !sameGrant(value, operation)) throw new Error(t("The saved permissions differ from this setup. Check the permissions in Client access before proceeding."));
      profile = value; stage = "profile"; config = null;
      refreshed = true;
      resumable = value.status === "unavailable" && value.canResumeRegistration === true;
      uncertain = value.status === "unavailable" && uncertain;
      if (!uncertain) operation = null;
      await configuration(request);
      await load();
    } catch (failure) {
      if (request !== generation) return;
      if (failure.status === 404 && !profile && operation?.scopes) {
        uncertain = false; operation = null; stage = "edit";
        notice = "No registration was found. Your draft is kept; review it before registering again.";
      } else {
        if (!refreshed && profile) { profile = { ...profile, status: "unavailable", reason: failure.message || String(failure) }; config = null; }
        error(failure, uncertain);
      }
    } finally {
      if (request === generation) { loading = false; render(); changed(); }
    }
  }

  async function submitRegistration(body, resume = false) {
    operation = body;
    const request = ++generation;
    busy = true; resumable = false; feedback = null; notice = ""; render(); changed();
    try {
      const value = validateProfile(await mutate("/api/local-clients", operation), operation.operationId);
      if (request !== generation) return;
      if (!sameGrant(value, operation)) throw new Error(t("Unexpected local agent response."));
      profile = value; operation = null; stage = "profile"; config = null; uncertain = false;
      selection.set(profile.id, targetId);
      await load(); await committed();
    } catch (failure) {
      if (request !== generation) return;
      uncertain = failure.mutationOutcome !== "not_applied";
      if (uncertain || resume) stage = "profile"; else operation = null;
      error(failure, uncertain);
    } finally { if (request === generation) { busy = false; render(); changed(); } }
  }

  async function register(event) {
    event.preventDefault();
    if (!editable() || inventory?.available !== true) return;
    draft.displayName = byId("local-client-name").value.trim();
    const availableScopes = new Set(inventory.scopeOptions.map(value => value.namespace));
    if (new TextEncoder().encode(draft.displayName).length > 120) { error(new Error(t("Agent name is too long."))); return; }
    if (!draft.displayName || (!draft.allScopes && (!draft.scopes.length || draft.scopes.some(scope => !availableScopes.has(scope))))) {
      error(new Error(t("Enter an agent name and choose at least one available scope."))); return;
    }
    return submitRegistration({ operationId: operationId(), ...descriptor(draft) });
  }

  async function resume() {
    if (!resumable || busy || loading || externalBusy || !profile) return;
    return submitRegistration({ operationId: profile.id, ...descriptor(profile) }, true);
  }

  async function approve() {
    if (busy || loading || uncertain || externalBusy || profile?.status !== "pending") return;
    const request = ++generation, id = profile.id;
    operation = { operationId: id, ...descriptor(profile) };
    busy = true; feedback = null; notice = ""; render(); changed();
    let applied = false;
    try {
      const value = validateProfile(await mutate(`${path(id)}/approve`), id);
      if (request !== generation) return;
      if (!sameGrant(value, operation)) throw new Error(t("Unexpected local agent response."));
      applied = true;
      profile = value; operation = null; config = null;
      await configuration(request);
      await load(); await committed();
    } catch (failure) {
      if (request !== generation) return;
      uncertain = !applied && failure.mutationOutcome !== "not_applied";
      if (!uncertain) operation = null;
      error(failure, uncertain);
    } finally { if (request === generation) { busy = false; render(); changed(); } }
  }

  async function close() {
    if (busy) return false;
    if (uncertain) {
      if (!await confirmAction({ title: t("Close and check later?"),
        description: t("The unknown setup outcome is kept while this Console stays open. Check the current state before authorizing again."),
        confirmLabel: t("Close and check later") })) return false;
    } else if (dirty()) {
      if (!await confirmAction({ title: t("Discard local agent draft?"), description: t("Your unregistered local agent draft will be discarded."), confirmLabel: t("Discard draft") })) return false;
      draft = newDraft();
    }
    generation++; loading = false; dialog.close(); changed(); return true;
  }

  async function openNew() {
    if (busy || externalBusy) return;
    if (!uncertain && !dirty()) {
      targetId = "generic";
      draft = newDraft(); stage = "edit"; profile = null; operation = null; config = null; resumable = false; feedback = null; notice = "";
      scopeQuery = ""; byId("local-client-scope-search").value = "";
      byId("local-client-name").value = ""; byId("local-client-mode").value = "read";
    }
    render(); if (!dialog.open) dialog.showModal();
    if (stage === "edit") byId("local-client-name").focus();
    if (!inventory) await load();
  }

  async function openProfile(id) {
    if (busy || (uncertain && id !== (profile?.id || operation?.operationId))) return;
    if (!uncertain) { targetId = localClientTarget(selection.get(id)).id; profile = inventory?.clients?.find(value => value.id === id) || null; operation = profile ? null : { operationId: id }; config = null; resumable = false; stage = "profile"; feedback = null; notice = ""; }
    render(); if (!dialog.open) dialog.showModal();
    await check();
  }

  targetSelect.addEventListener("change", () => {
    if (busy || loading || uncertain) return;
    targetId = localClientTarget(targetSelect.value).id;
    if (profile) selection.set(profile.id, targetId);
    notice = ""; render(false);
  });
  byId("local-client-add").addEventListener("click", openNew);
  byId("local-client-form").addEventListener("submit", register);
  byId("local-client-name").addEventListener("input", () => {
    if (!editable()) return;
    draft.displayName = byId("local-client-name").value; feedback = null; render(false); changed();
  });
  byId("local-client-mode").addEventListener("change", () => {
    if (!editable()) return;
    draft.mode = byId("local-client-mode").value === "contribute" ? "contribute" : "read";
    feedback = null; render(false); changed();
  });
  byId("local-client-scope-search").addEventListener("input", () => {
    scopeQuery = byId("local-client-scope-search").value.trim(); renderScopeOptions();
  });
  byId("local-client-scope-mode").addEventListener("change", () => {
    if (!editable()) return;
    draft.allScopes = byId("local-client-scope-mode").value === "all";
    feedback = null; render(); changed();
  });
  byId("local-client-approve").addEventListener("click", approve);
  byId("local-client-resume").addEventListener("click", resume);
  byId("local-client-check").addEventListener("click", check);
  byId("local-client-close").addEventListener("click", close);
  dialog.addEventListener("cancel", event => { event.preventDefault(); void close(); });
  function copyText(buttonId, textId, message) {
    byId(buttonId).addEventListener("click", async () => {
      if (!config || busy || loading || uncertain || profile?.status !== "ready" || byId(buttonId).disabled) return;
      const request = generation;
      try {
        await copy(byId(textId).value);
        if (request !== generation) return;
        notice = message; feedback = null; render(false);
      } catch (failure) {
        if (request !== generation) return;
        error(new Error(`${t("Could not copy. Select and copy the text below.")} ${failure.message || failure}`));
      }
    });
  }
  copyText("local-client-copy", "local-client-config", "MCP configuration copied.");
  copyText("local-client-copy-command", "local-client-install-command", "Install command copied.");
  copyText("local-client-copy-verification", "local-client-verification", "Verification prompt copied.");
  copyText("local-client-copy-guidance", "local-client-guidance", "Usage guidance copied.");
  byId("local-client-download").addEventListener("click", () => {
    if (!config || busy || loading || uncertain || profile?.status !== "ready" || byId("local-client-download").disabled) return;
    try { const value = exportValue(); download(value.text, value.filename, value.mimeType); }
    catch (failure) { error(failure); }
  });

  return {
    load, openNew, openProfile, close, statusLabel,
    snapshot: () => inventory,
    canOpenProfile: id => !busy && (!uncertain || id === (profile?.id || operation?.operationId)),
    blocksMutation: () => busy || uncertain,
    isMutating: () => busy,
    hasPendingAction: () => dirty() || busy || uncertain,
    setExternalBusy(value) { externalBusy = value; render(false); },
    rerender() { render(); },
  };
}
