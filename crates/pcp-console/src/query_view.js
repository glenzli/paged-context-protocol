export function createQueryView({ request, byId, element, showError, t, formatNumber, openPage }) {
  let capabilities = null;
  let method = "search";
  let busy = false;
  let scopeOptions = [];
  let result = null;

  function unavailableReason(value) {
    return capabilities?.unavailableMethods?.find((item) => item.method === value)?.reason || "";
  }

  function renderMethods() {
    for (const button of byId("context-query-methods").querySelectorAll("button[data-query-method]")) {
      const unavailable = Boolean(unavailableReason(button.dataset.queryMethod));
      const active = method === button.dataset.queryMethod;
      button.disabled = unavailable;
      button.setAttribute("aria-pressed", String(active));
      button.title = unavailable ? unavailableReason(button.dataset.queryMethod) : "";
    }
    byId("context-query-method-note").textContent = unavailableReason(method)
      || t("Search ranks literal matches and assembles the selected results without inference.");
  }

  function renderScopes() {
    const select = byId("context-query-scope");
    const selected = select.value;
    select.replaceChildren(
      new Option(t("All authorized scopes"), ""),
      ...scopeOptions.map((scope) => new Option(scope.displayName || scope.namespace, scope.namespace)),
    );
    select.value = scopeOptions.some((scope) => scope.namespace === selected) ? selected : "";
  }

  function detailLabel(detail) {
    return t({ payload: "Full payload", excerpt: "Excerpt", summary: "Current summary", reference: "Reference" }[detail] || "Reference");
  }

  function inclusionReason(entry) {
    if (entry.relation) {
      return `${t("Asserted relation")} ${entry.relation.relationType} (${relationDirection(entry.relation.direction)}) ${t("from anchor")} #${formatNumber(entry.anchorRank)} · ${t("Related context")}`;
    }
    const match = entry.matchedProjection || entry.matchedBy;
    if (entry.detail === "payload" || entry.detail === "excerpt") {
      return `${t("Literal match in")} ${match} · ${t("Ranked")} #${formatNumber(entry.rank)} · ${t("Focus layer")}`;
    }
    if (entry.detail === "summary") {
      return `${t("Ranked")} #${formatNumber(entry.rank)} · ${t("Summary layer")} · ${t("Current summary is tied to the selected revision.")}`;
    }
    return `${t("Ranked")} #${formatNumber(entry.rank)} · ${t("Reference layer")} · ${t("Included as a reference after detailed context.")}`;
  }

  function relationDirection(direction) {
    return t(direction === "incoming" ? "Incoming relation" : "Outgoing relation");
  }

  function entryRole(entry) {
    if (!entry.relation) return `${t("Anchor")} #${formatNumber(entry.anchorRank)}`;
    return `${t("Related context")} · ${entry.relation.relationType}`;
  }

  function renderModelContext() {
    const preview = byId("context-model-preview");
    if (!result) {
      preview.hidden = true;
      return;
    }
    preview.hidden = false;
    byId("context-model-context").textContent = result.modelContext || "";
    byId("context-model-context-status").textContent = `${formatNumber(result.entries?.filter((entry) => entry.content).length || 0)} ${t("context entries")}`;
  }

  function renderResult() {
    const target = byId("context-pack-results");
    if (!result) {
      target.replaceChildren(element("div", "empty", t("Run a query to inspect the ranked context pack.")));
      return;
    }
    const entries = result.entries || [];
    if (!entries.length) {
      target.replaceChildren(element("div", "empty", t("No results")));
      return;
    }
    target.replaceChildren(...entries.map((entry) => {
      const article = element("article", "context-pack-entry");
      const header = element("div", "context-pack-entry-heading");
      const identity = element("div", "context-pack-entry-identity");
      identity.append(
        element("strong", "", `#${formatNumber(entry.rank)} · ${entryRole(entry)} · ${detailLabel(entry.detail)}`),
        element("span", "muted", entry.namespace),
      );
      const actions = element("div", "context-pack-entry-actions");
      const open = element("button", "icon-button context-pack-reference-button", "↗");
      open.type = "button";
      open.title = t("Open source page");
      open.setAttribute("aria-label", t("Open source page"));
      open.addEventListener("click", () => openPage(entry.pageId).catch(showError));
      actions.append(open);
      header.append(identity, actions);
      article.append(header);
      if (entry.content) article.append(element("pre", "context-pack-entry-content", entry.content));
      article.append(element("div", "context-pack-entry-reason", `${t("Inclusion reason")}: ${inclusionReason(entry)}`));
      const evidence = element("details", "context-pack-entry-evidence");
      const summary = element("summary", "", t("Reference and provenance"));
      const evidenceText = element("div", "context-pack-entry-reference");
      evidenceText.append(document.createTextNode(`${entry.kind} · ${entry.namespace}`));
      if (entry.sourceProjectionTruncated) evidenceText.append(document.createTextNode(` · ${t("Source projection was incomplete; PCP downgraded it before packing.")}`));
      if (entry.sourceSpan) evidenceText.append(document.createTextNode(` · ${t("Source positions")} ${entry.sourceSpan.start}–${entry.sourceSpan.end}`));
      evidenceText.append(document.createTextNode(` · ${t("Revision")}: ${entry.revisionId}`));
      if (entry.provenanceRevisionIds?.length) evidenceText.append(document.createTextNode(` · ${t("Provenance")}: ${entry.provenanceRevisionIds.join(", ")}`));
      evidence.append(summary, evidenceText);
      article.append(evidence);
      return article;
    }));
  }

  function updateStatus() {
    if (!result) return;
    const visibility = result.visibility === "all_authorized" ? t("All authorized scopes") : result.scope;
    byId("query-status").textContent = `${formatNumber(result.anchorCount || 0)} ${t("anchors")} + ${formatNumber(result.relatedCount || 0)} ${t("related")} · ${visibility} · ${formatNumber(result.packBudgetChars)} ${t("char budget")}`;
  }

  async function load({ reload = false } = {}) {
    if (!capabilities || reload) capabilities = await request("/api/query/capabilities");
    if (!capabilities.availableMethods?.includes(method)) method = capabilities.availableMethods?.[0] || "search";
    renderMethods();
    renderScopes();
    renderResult();
    renderModelContext();
    updateStatus();
  }

  async function submit(event) {
    event.preventDefault();
    if (busy || unavailableReason(method)) return;
    const query = byId("context-query-text").value.trim();
    if (!query) return;
    busy = true;
    const submit = byId("context-query-submit");
    submit.disabled = true;
    byId("query-status").textContent = t("Loading");
    try {
      result = await request("/api/query", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          method,
          query,
          scope: byId("context-query-scope").value || null,
          topK: Number(byId("context-query-top-k").value),
        }),
      });
      renderResult();
      renderModelContext();
      updateStatus();
    } catch (error) {
      showError(error);
      byId("query-status").textContent = t("Load failed");
    } finally {
      busy = false;
      submit.disabled = false;
    }
  }

  byId("context-query-form").addEventListener("submit", (event) => submit(event).catch(showError));
  byId("context-query-methods").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-query-method]");
    if (!button || button.disabled) return;
    method = button.dataset.queryMethod;
    renderMethods();
  });

  return {
    async load(options) { await load(options); },
    setScopes(value) { scopeOptions = value || []; renderScopes(); },
    rerender() { renderMethods(); renderScopes(); renderResult(); renderModelContext(); updateStatus(); },
  };
}
