export function createRetentionView({ request, showError, formatNumber, formatTime, openPage, t }) {
  let loaded = false;
  let busy = false;
  let latest = null;

  const byId = (id) => document.getElementById(id);

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = String(text);
    return node;
  }

  function metric(label, value, tone = "", note = "") {
    const node = element("div", `metric${tone ? ` tone-${tone}` : ""}`);
    node.append(element("div", "metric-label", label), element("div", "metric-value", value));
    if (note) node.append(element("div", "metric-note", note));
    return node;
  }

  function formatBytes(bytes) {
    if (bytes < 1000) return `${formatNumber(bytes)} B`;
    if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB`;
    return `${(bytes / 1_000_000).toFixed(2)} MB`;
  }

  function reasonLabel(reason) {
    const label = ({
      current_head: "Current head",
      sealed_evidence: "Sealed evidence",
      recent_revision_window: "Recent Revision window",
      minimum_age_window: "Minimum age window",
      relation_endpoint: "Relation endpoint",
      relation_basis: "Relation basis",
      projection_head: "Projection head",
      summary_record: "Summary record",
      validity_record: "Validity record",
      idempotency_window: "Idempotency window",
      provenance_dependency: "Provenance dependency",
      explicit_lease: "Explicit retention lease",
      invalid_timestamp: "Invalid timestamp",
    })[reason];
    return label ? t(label) : reason;
  }

  function reasonDescription(reason, policy) {
    if (reason === "recent_revision_window") {
      const revisions = policy.keepRecentRevisionsPerPage;
      return `${t("Newest")} ${formatNumber(revisions)} ${t(revisions === 1 ? "Revision on each Page" : "Revisions on each Page")}`;
    }
    if (reason === "minimum_age_window") {
      const days = policy.minimumAgeDays;
      return `${t("Created less than")} ${formatNumber(days)} ${t(days === 1 ? "day ago" : "days ago")}`;
    }
    const description = ({
      current_head: "The Page version used by default reads",
      sealed_evidence: "Immutable source evidence",
      relation_endpoint: "Exact version recorded at a Relation endpoint",
      relation_basis: "Evidence used to assert a Relation",
      projection_head: "Current Summary or Validity projection",
      summary_record: "Referenced by a Summary record",
      validity_record: "Referenced by a Validity assessment",
      idempotency_window: "Needed to replay a recent write safely",
      provenance_dependency: "Input to another protected Revision",
      explicit_lease: "Held by a finite retention lease",
      invalid_timestamp: "Age cannot be proven safely",
    })[reason];
    return description ? t(description) : t("Store-defined protection root");
  }

  function pageButton(pageId, revisionId) {
    const button = element("button", "page-link");
    button.type = "button";
    button.append(element("span", "mono", pageId), element("span", "snippet mono", revisionId));
    button.addEventListener("click", () => openPage(pageId));
    return button;
  }

  function emptyRow(message, columns) {
    const row = document.createElement("tr");
    const cell = element("td", "empty", message);
    cell.colSpan = columns;
    row.append(cell);
    return row;
  }

  function renderReasons(plan) {
    const rows = (plan.protectionReasons || []).map((entry) => {
      const row = document.createElement("tr");
      const share = plan.scannedRevisions
        ? `${Math.round((entry.revisions / plan.scannedRevisions) * 100)}%`
        : "-";
      row.append(
        element("td", "", reasonLabel(entry.reason)),
        element("td", "muted", reasonDescription(entry.reason, plan.policy)),
        element("td", "", formatNumber(entry.revisions)),
        element("td", "muted", share),
      );
      return row;
    });
    byId("retention-reason-rows").replaceChildren(...(rows.length ? rows : [emptyRow(t("No protection roots"), 4)]));
  }

  function renderCandidates(plan) {
    const rows = (plan.candidates || []).map((candidate) => {
      const row = document.createElement("tr");
      const page = element("td");
      page.append(pageButton(candidate.pageId, candidate.revisionId));
      row.append(
        page,
        element("td", "mono", candidate.namespace),
        element("td", "", candidate.kind),
        element("td", "", formatTime(candidate.createdAt)),
        element("td", "", formatBytes(candidate.estimatedBytes)),
      );
      return row;
    });
    byId("retention-candidate-rows").replaceChildren(...(rows.length ? rows : [emptyRow(t("No reclaimable historical Revisions under this policy"), 5)]));
    byId("retention-candidate-section").hidden = plan.candidateRevisions === 0;
    byId("retention-candidate-status").textContent = plan.candidatesTruncated
      ? `${formatNumber(plan.candidates.length)} ${t("shown of")} ${formatNumber(plan.candidateRevisions)}`
      : `${formatNumber(plan.candidateRevisions)} ${t("total")}`;
  }

  function renderProtected(plan) {
    const rows = (plan.protectedSamples || []).map((sample) => {
      const row = document.createElement("tr");
      const page = element("td");
      page.append(pageButton(sample.pageId, sample.revisionId));
      row.append(
        page,
        element("td", "mono", sample.namespace),
        element("td", "", sample.kind),
        element("td", "retention-reasons", sample.reasons.map(reasonLabel).join(" · ")),
        element("td", "", formatTime(sample.createdAt)),
      );
      return row;
    });
    byId("retention-protected-rows").replaceChildren(...(rows.length ? rows : [emptyRow(t("No protected historical samples"), 5)]));
    const protectedHistory = Math.max(0, plan.protectedRevisions - protectionCount(plan, "current_head"));
    byId("retention-protected-status").textContent = plan.protectedSamplesTruncated
      ? `${formatNumber(plan.protectedSamples.length)} ${t("shown of")} ${formatNumber(protectedHistory)}`
      : `${formatNumber(protectedHistory)} ${t("total")}`;
  }

  function renderLeases(leases, total) {
    const rows = leases.map((lease) => {
      const row = document.createElement("tr");
      const page = element("td");
      page.append(pageButton(lease.pageId, lease.revisionId));
      row.append(
        page,
        element("td", "mono", lease.namespace),
        element("td", "mono", lease.holderPrincipalId),
        element("td", "retention-reasons", lease.reason),
        element("td", "", formatTime(lease.expiresAt)),
      );
      return row;
    });
    byId("retention-lease-section").hidden = total === 0;
    byId("retention-lease-rows").replaceChildren(...(rows.length ? rows : [emptyRow(t("No active temporary holds"), 5)]));
    byId("retention-lease-status").textContent = leases.length < total
      ? `${formatNumber(leases.length)} ${t("shown of")} ${formatNumber(total)} ${t("active")}`
      : `${formatNumber(total)} ${t("active")}`;
  }

  function protectionCount(plan, reason) {
    return (plan.protectionReasons || []).find((entry) => entry.reason === reason)?.revisions || 0;
  }

  function render({ plan, leases = [] }) {
    latest = { plan, leases };
    const hasCandidates = plan.candidateRevisions > 0;
    const currentHeads = protectionCount(plan, "current_head");
    const protectedHistory = Math.max(0, plan.protectedRevisions - currentHeads);
    const metrics = byId("retention-metrics");
    metrics.hidden = false;
    metrics.replaceChildren(
      metric(t("Reclaimable history"), formatNumber(plan.candidateRevisions), hasCandidates ? "warning" : "positive", `${formatNumber(plan.candidatePages)} ${t("Pages")}`),
      metric(t("Estimated storage"), formatBytes(plan.candidateEstimatedBytes), hasCandidates ? "warning" : "positive", t("Reclaimable historical revision content")),
      metric(t("Protected history"), formatNumber(protectedHistory), "positive", t("Historical revisions still required by a protection root")),
      metric(t("Active temporary holds"), formatNumber(plan.activeRetentionLeases), plan.activeRetentionLeases ? "info" : "", t("Finite holds on revision history")),
    );

    const outcome = element(
      "div",
      `health-signal tone-${hasCandidates ? "warning" : "positive"}`,
      hasCandidates
        ? `${formatNumber(plan.candidateRevisions)} ${t("historical Revisions may be reclaimable under current safeguards.")}`
        : `${t("No historical Revision is reclaimable under current safeguards.")} ${formatNumber(protectedHistory)} ${t("historical Revisions remain protected.")}`,
    );
    const boundary = element(
      "div",
      "health-signal tone-info",
      t("This check is read-only. No revision is deleted from the Console."),
    );
    const mode = byId("retention-mode");
    mode.hidden = false;
    mode.replaceChildren(outcome, boundary);
    renderLeases(leases, plan.activeRetentionLeases);
    renderReasons(plan);
    renderCandidates(plan);
    renderProtected(plan);
    byId("retention-status").textContent = `${plan.scopes.length} ${t(plan.scopes.length === 1 ? "scope" : "scopes")} · ${t("preview updated")} ${new Date(plan.generatedAt).toLocaleTimeString()}`;
  }

  async function load({ reload = false } = {}) {
    if (busy || (loaded && !reload)) return;
    busy = true;
    byId("retention-status").textContent = t("Planning");
    try {
      render(await request("/api/retention"));
      loaded = true;
    } catch (error) {
      showError(error);
      byId("retention-status").textContent = t("Plan failed");
    } finally {
      busy = false;
    }
  }

  byId("retention-details").addEventListener("toggle", () => {
    if (byId("retention-details").open) load().catch(showError);
  });

  async function refreshIfOpen() {
    if (byId("retention-details").open) await load({ reload: true });
  }

  function rerender() {
    if (latest) render(latest);
  }

  return { refreshIfOpen, rerender };
}
