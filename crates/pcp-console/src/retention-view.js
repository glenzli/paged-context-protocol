export function createRetentionView({ request, showError, formatNumber, formatTime, openPage }) {
  let loaded = false;
  let busy = false;
  let capabilities = null;

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
    return ({
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
    })[reason] || reason;
  }

  function reasonDescription(reason, policy) {
    return ({
      current_head: "The Page version used by default reads",
      sealed_evidence: "Immutable source evidence",
      recent_revision_window: `Newest ${policy.keepRecentRevisionsPerPage} Revision${policy.keepRecentRevisionsPerPage === 1 ? "" : "s"} on each Page`,
      minimum_age_window: `Created less than ${policy.minimumAgeDays} day${policy.minimumAgeDays === 1 ? "" : "s"} ago`,
      relation_endpoint: "Exact version recorded at a Relation endpoint",
      relation_basis: "Evidence used to assert a Relation",
      projection_head: "Current Summary or Validity projection",
      summary_record: "Referenced by a Summary record",
      validity_record: "Referenced by a Validity assessment",
      idempotency_window: "Needed to replay a recent write safely",
      provenance_dependency: "Input to another protected Revision",
      explicit_lease: "Held by a finite retention lease",
      invalid_timestamp: "Age cannot be proven safely",
    })[reason] || "Store-defined protection root";
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
    byId("retention-reason-rows").replaceChildren(...(rows.length ? rows : [emptyRow("No protection roots", 4)]));
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
    byId("retention-candidate-rows").replaceChildren(...(rows.length ? rows : [emptyRow("No eligible historical Revisions under this policy", 5)]));
    byId("retention-candidate-status").textContent = plan.candidatesTruncated
      ? `${formatNumber(plan.candidates.length)} shown of ${formatNumber(plan.candidateRevisions)}`
      : `${formatNumber(plan.candidateRevisions)} total`;
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
    byId("retention-protected-rows").replaceChildren(...(rows.length ? rows : [emptyRow("No protected historical samples", 5)]));
    const protectedHistory = Math.max(0, plan.protectedRevisions - protectionCount(plan, "current_head"));
    byId("retention-protected-status").textContent = plan.protectedSamplesTruncated
      ? `${formatNumber(plan.protectedSamples.length)} shown of ${formatNumber(protectedHistory)}`
      : `${formatNumber(protectedHistory)} total`;
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
    byId("retention-lease-rows").replaceChildren(...(rows.length ? rows : [emptyRow("No active explicit retention leases", 5)]));
    byId("retention-lease-status").textContent = leases.length < total
      ? `${formatNumber(leases.length)} shown of ${formatNumber(total)} active`
      : `${formatNumber(total)} active`;
  }

  function protectionCount(plan, reason) {
    return (plan.protectionReasons || []).find((entry) => entry.reason === reason)?.revisions || 0;
  }

  function render({ plan, leases = [] }) {
    const hasCandidates = plan.candidateRevisions > 0;
    const currentHeads = protectionCount(plan, "current_head");
    const protectedHistory = Math.max(0, plan.protectedRevisions - currentHeads);
    byId("retention-metrics").replaceChildren(
      metric("Scanned Revisions", formatNumber(plan.scannedRevisions), "", `${formatNumber(plan.scannedPages)} stable Pages`),
      metric("Current heads", formatNumber(currentHeads), "positive", "Always protected"),
      metric("Protected history", formatNumber(protectedHistory), "positive", "Historical versions still required"),
      metric("Eligible history", formatNumber(plan.candidateRevisions), hasCandidates ? "warning" : "positive", `${formatBytes(plan.candidateEstimatedBytes)} across ${formatNumber(plan.candidatePages)} Pages`),
    );

    const collectionAvailable = Boolean(capabilities?.features?.includes("revision_retention"));
    const outcome = element(
      "div",
      `health-signal tone-${hasCandidates ? "warning" : "positive"}`,
      hasCandidates
        ? `${formatNumber(plan.candidateRevisions)} historical Revisions are eligible because they predate ${formatTime(plan.cutoffAt)} and have no protection root.`
        : `No historical Revision is eligible under this policy. ${formatNumber(protectedHistory)} historical Revisions remain protected.`,
    );
    const boundary = element(
      "div",
      "health-signal tone-info",
      collectionAvailable
        ? "This Console is dry-run only. The Runtime supports collection through a separately authorized admin client."
        : "This Console is dry-run only, and the Runtime does not advertise physical collection.",
    );
    byId("retention-mode").replaceChildren(outcome, boundary);
    renderLeases(leases, plan.activeRetentionLeases);
    renderReasons(plan);
    renderCandidates(plan);
    renderProtected(plan);
    byId("retention-status").textContent = `${plan.scopes.length} scope${plan.scopes.length === 1 ? "" : "s"} · dry run updated ${new Date(plan.generatedAt).toLocaleTimeString()}`;
  }

  function setScopes(scopes) {
    const selector = byId("retention-scope");
    const current = selector.value;
    const all = element("option", "", "All scopes");
    all.value = "";
    const options = (scopes || []).map((scope) => {
      const option = element("option", "", scope.displayName || scope.namespace);
      option.value = scope.namespace;
      return option;
    });
    selector.replaceChildren(all, ...options);
    selector.value = options.some((option) => option.value === current) ? current : "";
  }

  function setCapabilities(value) {
    capabilities = value;
  }

  async function load({ reload = false } = {}) {
    if (busy || (loaded && !reload)) return;
    busy = true;
    byId("retention-run").disabled = true;
    byId("retention-status").textContent = "Planning";
    try {
      const params = new URLSearchParams(new FormData(byId("retention-controls")));
      render(await request(`/api/retention?${params}`));
      loaded = true;
    } catch (error) {
      showError(error);
      byId("retention-status").textContent = "Plan failed";
    } finally {
      busy = false;
      byId("retention-run").disabled = false;
    }
  }

  byId("retention-controls").addEventListener("submit", (event) => {
    event.preventDefault();
    load({ reload: true }).catch(showError);
  });

  return { load, setScopes, setCapabilities };
}
