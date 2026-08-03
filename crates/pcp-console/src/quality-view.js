const LONG_PAGE_CHARS = 4_000;

export function createQualityView({ request, showError, formatNumber, openPage }) {
  let loaded = false;
  let busy = false;

  const byId = (id) => document.getElementById(id);

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = String(text);
    return node;
  }

  function metric(label, value, tone = "") {
    const node = element("div", `metric${tone ? ` tone-${tone}` : ""}`);
    node.append(element("div", "metric-label", label), element("div", "metric-value", value));
    return node;
  }

  function isSummaryPage(item) {
    return item.kind === "summary" || item.kind === "page_summary";
  }

  function issuesFor(item) {
    const issues = [];
    if (!isSummaryPage(item) && item.contentChars >= LONG_PAGE_CHARS && !item.summaryPageId) {
      issues.push("Long Page has no Summary");
    }
    if (item.relationTypes.length > 12) issues.push("Dense relation neighborhood");
    if (!item.relationTypes.length && !["conversation_event", "attachment"].includes(item.kind)) {
      issues.push("No graph relations");
    }
    return issues;
  }

  function render(data) {
    const items = data.items || [];
    const summaryEligible = items.filter((item) => !isSummaryPage(item) && item.contentChars >= LONG_PAGE_CHARS);
    const summarized = summaryEligible.filter((item) => item.summaryPageId);
    const linked = items.filter((item) => item.relationTypes.length);
    const coverage = summaryEligible.length ? `${Math.round((summarized.length / summaryEligible.length) * 100)}%` : "-";
    const relationCoverage = items.length ? `${Math.round((linked.length / items.length) * 100)}%` : "-";
    byId("quality-metrics").replaceChildren(
      metric("Recent active Pages", formatNumber(items.length)),
      metric("Long Pages", formatNumber(summaryEligible.length)),
      metric("Long-page Summary", coverage, summaryEligible.length && summarized.length < summaryEligible.length ? "warning" : "positive"),
      metric("Relation coverage", relationCoverage, "info"),
    );

    const findings = items
      .map((item) => ({ item, issues: issuesFor(item) }))
      .filter((entry) => entry.issues.length);
    const rows = findings.map(({ item, issues }) => {
      const row = document.createElement("tr");
      const pageCell = element("td");
      const open = element("button", "page-link");
      open.type = "button";
      open.append(
        element("span", "mono", item.pageId),
        element("span", "snippet", item.summary || item.snippet || "No preview"),
      );
      open.addEventListener("click", () => openPage(item.pageId));
      pageCell.append(open);
      row.append(
        pageCell,
        element("td", "mono", item.namespace),
        element("td", "", formatNumber(item.contentChars)),
        element("td", "quality-issue", issues.join(" · ")),
      );
      return row;
    });
    if (!rows.length) {
      const row = document.createElement("tr");
      const cell = element("td", "empty", "No issues in the recent active sample");
      cell.colSpan = 4;
      row.append(cell);
      rows.push(row);
    }
    byId("quality-rows").replaceChildren(...rows);
    byId("quality-status").textContent = `${formatNumber(items.length)} most recent active Pages sampled`;
  }

  async function load({ reload = false } = {}) {
    if (busy || (loaded && !reload)) return;
    busy = true;
    byId("quality-status").textContent = "Loading";
    try {
      render(await request("/api/quality"));
      loaded = true;
    } catch (error) {
      showError(error);
      byId("quality-status").textContent = "Load failed";
    } finally {
      busy = false;
    }
  }

  return { load };
}
