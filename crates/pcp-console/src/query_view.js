export function buildQueryRequest({ method, query, scope, topK, intentEffort }) {
  const payload = { query, scopes: scope ? [scope] : [], resultLimit: Number(topK) };
  if (method === "match_intent") payload.intentEffort = intentEffort;
  return payload;
}

export function createQueryView({ request, byId, element, showError, t, formatNumber, openPage, openPageIcon }) {
  let method = "semantic_search";
  let busy = false;
  let busyStartedAt = null;
  let busyTimer = null;
  let scopeOptions = [];
  let result = null;
  let audit = null;

  function renderMethods() {
    const methodSelect = byId("context-query-method");
    methodSelect.value = method;
    methodSelect.disabled = busy;
    byId("context-query-effort-control").hidden = method !== "match_intent";
    byId("context-query-effort").disabled = busy;
    byId("context-query-scope").disabled = busy;
    byId("context-query-top-k").disabled = busy;
    byId("context-query-text").disabled = busy;
    byId("context-query-method-note").textContent = method === "match_intent"
      ? t("Intent matching lets the Router expand and review bounded candidates before it assembles a context pack.")
      : t("Semantic retrieval returns independently relevant pages; asserted structure only makes bounded ranking adjustments.");
    const submit = byId("context-query-submit");
    submit.disabled = busy;
    submit.textContent = busy
      ? t(method === "match_intent" ? "Matching intent…" : "Searching context…")
      : t("Build context pack");
    submit.classList.toggle("is-loading", busy);
    submit.setAttribute("aria-busy", busy ? "true" : "false");
  }

  function clearQueryError() {
    const box = byId("context-query-error");
    box.hidden = true;
    byId("context-query-error-message").textContent = "";
  }

  function showQueryError(error) {
    const message = error?.message || String(error);
    byId("context-query-error-message").textContent = message;
    byId("context-query-error").hidden = false;
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
    if (entry.intentReason) {
      return `${t("Router selection")}: ${entry.intentReason} · ${t("Ranked")} #${formatNumber(entry.rank)} · ${t("Focus layer")}`;
    }
    const match = entry.matchedProjection || entry.matchedBy;
    if (entry.semanticScore != null) {
      const structural = entry.structuralBoost == null
        ? ""
        : ` · ${t("Structure boost")} +${Number(entry.structuralBoost).toFixed(3)}${entry.structuralRelations?.length ? ` ${t("via")} ${entry.structuralRelations.map((relation) => relation.relationType).join(", ")}` : ""}`;
      return `${t("Vector similarity")} ${Number(entry.semanticScore).toFixed(3)}${structural} · ${t("Ranked")} #${formatNumber(entry.rank)} · ${t("Focus layer")}`;
    }
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
    byId("context-model-context").textContent = (result.entries || [])
      .filter((entry) => entry.content)
      .map((entry) => entry.content)
      .join("\n\n");
    byId("context-model-context-status").textContent = `${formatNumber(result.entries?.filter((entry) => entry.content).length || 0)} ${t("context entries")}`;
  }

  function auditMetric(label, value) {
    const node = element("div", "context-query-audit-metric");
    node.append(element("span", "", label), element("strong", "", value));
    return node;
  }

  function renderAudit() {
    const target = byId("context-query-audit");
    if (!audit) {
      target.hidden = true;
      target.replaceChildren();
      return;
    }
    target.hidden = false;
    const heading = element("div", "context-query-audit-heading");
    heading.append(
      element("h3", "", t("Query activity")),
      element("span", "", `${formatNumber(audit.windowHours || 24)} ${t("hours")} · ${t("privacy-preserving")}`),
    );
    const usage = audit.routerUsage || {};
    const metrics = element("div", "context-query-audit-metrics");
    metrics.append(
      auditMetric(t("Calls"), formatNumber(audit.calls || 0)),
      auditMetric(t("Semantic calls"), formatNumber(audit.semanticSearch?.calls || 0)),
      auditMetric(t("Intent calls"), formatNumber(audit.matchIntent?.calls || 0)),
      auditMetric(t("Query Router tokens"), formatNumber(usage.totalTokens || 0)),
    );
    target.replaceChildren(heading, metrics);
    const recent = audit.recentEvents || [];
    if (recent.length) {
      const details = element("details", "context-query-audit-recent");
      const summary = element("summary", "", `${t("Recent query calls")} · ${formatNumber(recent.length)}`);
      const list = element("ul", "");
      recent.forEach((event) => {
        const methodLabel = event.method === "match_intent" ? t("Intent matching") : t("Semantic search");
        const tokens = event.routerUsage?.totalTokens ? ` · ${formatNumber(event.routerUsage.totalTokens)} ${t("tokens")}` : "";
        const failure = event.failureKind ? ` · ${event.failureKind}` : "";
        list.append(element("li", "", `${event.occurredAt} · ${methodLabel} · ${formatNumber(event.durationMs || 0)} ms · ${formatNumber(event.anchorCount || 0)} ${t("anchors")}${tokens}${failure}`));
      });
      details.append(summary, list);
      target.append(details);
    }
  }

  async function loadAudit() {
    const scope = byId("context-query-scope").value;
    const params = new URLSearchParams({ hours: "24" });
    if (scope) params.set("scope", scope);
    try {
      audit = await request(`/api/query/audit?${params}`);
    } catch (_) {
      // Query auditing is diagnostic. A client without Audit may still use
      // retrieval, and the main query surface must remain usable.
      audit = null;
    }
    renderAudit();
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
      const open = element("button", "icon-button context-pack-reference-button");
      open.type = "button";
      open.title = t("Open source page");
      open.setAttribute("aria-label", t("Open source page"));
      open.append(openPageIcon());
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
    if (result.intentMatch) {
      const audit = result.intentMatch;
      const details = element("details", "context-pack-entry-evidence");
      const summary = element("summary", "", t("Intent match audit"));
      const text = element("div", "context-pack-entry-reference");
      const usage = audit.routerUsage;
      const usageText = !usage?.reportedResponses
        ? `${t("Router token usage")}: ${t("Provider did not report token usage.")}`
        : `${t("Router token usage")}: ${formatNumber(usage.totalTokens || 0)} ${t("tokens")} (${t("Input tokens")} ${formatNumber(usage.inputTokens || 0)} · ${t("Output tokens")} ${formatNumber(usage.outputTokens || 0)}${usage.cachedInputTokens ? ` · ${t("Cached tokens")} ${formatNumber(usage.cachedInputTokens)}` : ""}${usage.reasoningTokens ? ` · ${t("Reasoning tokens")} ${formatNumber(usage.reasoningTokens)}` : ""}) · ${t("Reported responses")} ${formatNumber(usage.reportedResponses)}/${formatNumber((usage.reportedResponses || 0) + (usage.unreportedResponses || 0))}`;
      text.textContent = `${t("Retrieval effort")}: ${audit.effort} · ${t("Router rounds")}: ${formatNumber(audit.routerRounds || 0)} · ${usageText} · ${t("Candidates")}: ${formatNumber(audit.candidateCount || 0)} · ${t("Consulted Pages")}: ${formatNumber(audit.consultedCount || 0)} · ${t("Relation leads reviewed")}: ${formatNumber(audit.relationCandidatesConsidered || 0)} · ${t("Catalog Pages considered")}: ${formatNumber(audit.catalogPagesConsidered || 0)} · ${t("Stopped")}: ${audit.stoppedReason || ""}`;
      details.append(summary, text);
      target.append(details);
    }
  }

  function busyMessage() {
    const elapsedSeconds = busyStartedAt == null
      ? 0
      : Math.floor((Date.now() - busyStartedAt) / 1000);
    const action = method === "match_intent"
      ? t("Intent match in progress")
      : t("Semantic search in progress");
    return `${action} · ${formatNumber(elapsedSeconds)} ${t("seconds")}`;
  }

  function busyDetail() {
    const action = method === "match_intent"
      ? t("The Router is retrieving and reviewing bounded candidates.")
      : t("Finding independently relevant pages.");
    const stability = result
      ? t("The previous completed context pack stays visible until this atomic request completes.")
      : t("Ranking and budget assembly return as one stable context pack; unstable intermediate ranks are not shown.");
    return `${action} ${stability}`;
  }

  function renderBusyPresentation() {
    const form = byId("context-query-form");
    const target = byId("context-pack-results");
    form.setAttribute("aria-busy", busy ? "true" : "false");
    target.setAttribute("aria-busy", busy ? "true" : "false");
    const existing = byId("context-query-progress");
    if (!busy) {
      existing?.remove();
      return;
    }
    const progress = existing || element("div", "context-query-progress");
    progress.id = "context-query-progress";
    progress.setAttribute("role", "status");
    progress.setAttribute("aria-live", "polite");
    const copy = element("span", "context-query-progress-copy");
    copy.append(element("strong", "", busyMessage()), element("span", "", busyDetail()));
    progress.replaceChildren(element("span", "context-query-progress-indicator"), copy);
    if (!existing) target.prepend(progress);
    byId("query-status").textContent = busyMessage();
  }

  function setBusy(next) {
    busy = next;
    if (busy) {
      busyStartedAt = Date.now();
      busyTimer = window.setInterval(() => renderBusyPresentation(), 1000);
    } else {
      if (busyTimer != null) window.clearInterval(busyTimer);
      busyTimer = null;
      busyStartedAt = null;
    }
    renderMethods();
    renderBusyPresentation();
  }

  function updateStatus() {
    if (!result) return;
    const visibility = result.visibility === "all_authorized"
      ? t("All authorized scopes")
      : (result.scopes || []).join(" · ");
    const semantic = result.semanticIndexedCount == null ? "" : ` · ${formatNumber(result.semanticIndexedCount)} ${t("vector documents")} (${formatNumber(result.semanticEmbeddedCount || 0)} ${t("new")})`;
    const related = Number(result.relatedCount || 0);
    const relationStatus = related ? ` + ${formatNumber(related)} ${t("related")}` : "";
    byId("query-status").textContent = `${formatNumber(result.anchorCount || 0)} ${t("anchors")}${relationStatus} · ${visibility} · ${formatNumber(result.contextBudgetChars)} ${t("char budget")}${semantic}`;
  }

  async function load() {
    renderMethods();
    renderScopes();
    renderResult();
    renderModelContext();
    await loadAudit();
    updateStatus();
    renderBusyPresentation();
  }

  async function submit(event) {
    event.preventDefault();
    if (busy) return;
    const query = byId("context-query-text").value.trim();
    if (!query) return;
    clearQueryError();
    setBusy(true);
    try {
      const payload = buildQueryRequest({
        method,
        query,
        scope: byId("context-query-scope").value || null,
        topK: Number(byId("context-query-top-k").value),
        intentEffort: byId("context-query-effort").value,
      });
      const endpoint = method === "match_intent"
        ? "/api/query/match-intent"
        : "/api/query/semantic-search";
      result = await request(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      renderResult();
      renderModelContext();
      updateStatus();
      await loadAudit();
    } catch (error) {
      showQueryError(error);
      byId("query-status").textContent = t("Query failed");
    } finally {
      setBusy(false);
    }
  }

  byId("context-query-form").addEventListener("submit", (event) => submit(event).catch(showError));
  byId("context-query-method").addEventListener("change", (event) => {
    method = event.target.value;
    clearQueryError();
    renderMethods();
  });
  byId("context-query-scope").addEventListener("change", () => loadAudit().catch(showError));
  byId("context-query-retry").addEventListener("click", () => {
    byId("context-query-form").requestSubmit();
  });

  return {
    async load(options) { await load(options); },
    setScopes(value) { scopeOptions = value || []; renderScopes(); },
    rerender() { renderMethods(); renderScopes(); renderResult(); renderModelContext(); renderAudit(); updateStatus(); renderBusyPresentation(); },
  };
}
