export function createHealthView({ request, showError, formatNumber }) {
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

  function percentage(part, total) {
    return total ? `${Math.round((part / total) * 100)}%` : "-";
  }

  function duration(value) {
    if (value === null || value === undefined) return "Collecting";
    if (value < 1000) return `${formatNumber(value)} ms`;
    return `${(value / 1000).toFixed(2)} s`;
  }

  function outputSize(bytes) {
    if (bytes < 1000) return `${formatNumber(bytes)} B`;
    if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB`;
    return `${(bytes / 1_000_000).toFixed(2)} MB`;
  }

  function panel(label, entries) {
    const node = element("section", "aggregate-panel");
    node.append(element("h3", "", label));
    for (const [name, value, tone] of entries) {
      const row = element("div", `aggregate-row${tone ? ` health-${tone}` : ""}`);
      row.append(element("span", "", name), element("strong", "", value));
      node.append(row);
    }
    return node;
  }

  function renderSignals(data) {
    const signals = [];
    if (!data.activity.calls) {
      signals.push(["info", "No workload calls were observed in this window. Telemetry begins with the upgraded runtime."]);
    }
    if (data.activity.calls && data.activity.measuredCalls < data.activity.calls) {
      signals.push(["info", `${percentage(data.activity.measuredCalls, data.activity.calls)} of calls in this window include detailed telemetry; recall ratios use only that measured sample.`]);
    }
    if (data.recall.searches) {
      const zeroRate = data.recall.zeroResultSearches / data.recall.searches;
      if (zeroRate >= 0.35) signals.push(["warning", `${percentage(data.recall.zeroResultSearches, data.recall.searches)} of searches returned no Page.`]);
      else signals.push(["positive", `${percentage(data.recall.searches - data.recall.zeroResultSearches, data.recall.searches)} of searches returned at least one Page.`]);
    }
    if (data.storage.longPages) {
      const coverage = data.storage.summarizedLongPages / data.storage.longPages;
      signals.push([
        coverage >= 0.7 ? "positive" : "warning",
        `${percentage(data.storage.summarizedLongPages, data.storage.longPages)} of long current Pages have a Summary route.`,
      ]);
    }
    if (data.consolidation.runs) {
      signals.push(["info", `Consolidation removed ${formatNumber(data.consolidation.netPageReduction)} redundant current Page${data.consolidation.netPageReduction === 1 ? "" : "s"} in this window.`]);
    }
    if (data.activity.failed || data.activity.denied) {
      signals.push(["danger", `${formatNumber(data.activity.failed)} failed and ${formatNumber(data.activity.denied)} denied calls need inspection.`]);
    }
    const nodes = signals.map(([tone, text]) => element("div", `health-signal tone-${tone}`, text));
    byId("health-signals").replaceChildren(...nodes);
  }

  function renderTimeline(data) {
    const buckets = data.timeline || [];
    if (!buckets.length) {
      byId("health-timeline").replaceChildren(element("div", "empty", "No workload activity in this window"));
      return;
    }
    const maximum = Math.max(...buckets.map((bucket) => bucket.calls), 1);
    const chart = element("div", "timeline-chart");
    for (const bucket of buckets) {
      const item = element("div", "timeline-bucket");
      const bars = element("div", "timeline-bars");
      const calls = element("div", "timeline-bar calls");
      calls.style.height = `${Math.max(3, Math.round((bucket.calls / maximum) * 72))}px`;
      calls.title = `${bucket.calls} calls`;
      const failures = element("div", "timeline-bar failures");
      failures.style.height = `${Math.round((bucket.failures / maximum) * 72)}px`;
      failures.title = `${bucket.failures} failures`;
      bars.append(calls, failures);
      const timestamp = new Date(bucket.bucket);
      const label = Number.isNaN(timestamp.getTime())
        ? bucket.bucket.slice(5)
        : (data.windowHours <= 48
          ? timestamp.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
          : timestamp.toLocaleDateString([], { month: "numeric", day: "numeric" }));
      item.append(bars, element("span", "timeline-label", label));
      chart.append(item);
    }
    byId("health-timeline").replaceChildren(chart);
  }

  function renderOperations(data) {
    const rows = [...(data.operations || [])]
      .sort((left, right) => right.calls - left.calls || left.operation.localeCompare(right.operation))
      .map((operation) => {
        const row = document.createElement("tr");
        row.append(
          element("td", "mono", operation.operation),
          element("td", "", operation.measuredCalls === operation.calls
            ? formatNumber(operation.calls)
            : `${formatNumber(operation.calls)} · ${formatNumber(operation.measuredCalls)} measured`),
          element("td", operation.failures ? "health-danger" : "", formatNumber(operation.failures)),
          element("td", "", `${formatNumber(operation.outputCount)} · ${outputSize(operation.outputBytes)}`),
          element("td", "", duration(operation.p50DurationMs)),
          element("td", "", duration(operation.p95DurationMs)),
        );
        return row;
      });
    if (!rows.length) {
      const row = document.createElement("tr");
      const cell = element("td", "empty", "No workload operations in this window");
      cell.colSpan = 6;
      row.append(cell);
      rows.push(row);
    }
    byId("health-operation-rows").replaceChildren(...rows);
  }

  function renderScopes(data) {
    const rows = (data.scopes || []).map((scope) => {
      const row = document.createElement("tr");
      row.append(
        element("td", "mono", scope.namespace),
        element("td", "", formatNumber(scope.currentPages)),
        element("td", "", formatNumber(scope.pages)),
        element("td", "", formatNumber(scope.revisions)),
        element("td", "", formatNumber(scope.calls)),
        element("td", "", formatNumber(scope.searches)),
        element("td", "", formatNumber(scope.writes)),
      );
      return row;
    });
    byId("health-scope-rows").replaceChildren(...rows);
  }

  function render(data) {
    const failureCount = data.activity.failed + data.activity.denied;
    byId("health-metrics").replaceChildren(
      metric("Current Pages", formatNumber(data.storage.currentPages)),
      metric("New current Pages", `+${formatNumber(data.storage.currentPagesCreated)}`, data.storage.currentPagesCreated ? "info" : ""),
      metric("Workload calls", formatNumber(data.activity.calls)),
      metric("Call exceptions", formatNumber(failureCount), failureCount ? "danger" : "positive"),
    );
    byId("health-flows").replaceChildren(
      panel("Recall", [
        ["Searches", formatNumber(data.recall.searches)],
        ["Zero-result", percentage(data.recall.zeroResultSearches, data.recall.searches), data.recall.zeroResultSearches ? "warning" : "positive"],
        ["Pages returned", formatNumber(data.recall.returnedPages)],
        ["Summary / detail reads", `${formatNumber(data.recall.summaryReads)} / ${formatNumber(data.recall.detailReads)}`],
      ]),
      panel("Consolidation", [
        ["Runs", formatNumber(data.consolidation.runs)],
        ["Input Pages", formatNumber(data.consolidation.inputPages)],
        ["Net contraction", formatNumber(data.consolidation.netPageReduction), data.consolidation.netPageReduction ? "positive" : ""],
        ["Historical Revisions", formatNumber(data.storage.historicalRevisions)],
      ]),
      panel("Shape", [
        ["Pages / Revisions", `${formatNumber(data.storage.pages)} / ${formatNumber(data.storage.revisions)}`],
        ["Sealed / revisioned", `${formatNumber(data.storage.sealedPages)} / ${formatNumber(data.storage.revisionedPages)}`],
        ["Relations", formatNumber(data.graph.relations)],
        ["Relations / Page", data.graph.averageRelationsPerPage.toFixed(1)],
      ]),
      panel("Runtime", [
        ["Principals", formatNumber(data.activity.principals)],
        ["p50 latency", duration(data.activity.p50DurationMs)],
        ["p95 latency", duration(data.activity.p95DurationMs)],
        ["Long-page Summary", percentage(data.storage.summarizedLongPages, data.storage.longPages)],
      ]),
    );
    renderSignals(data);
    renderTimeline(data);
    renderOperations(data);
    renderScopes(data);
    byId("health-status").textContent = `${data.windowHours}h window · updated ${new Date(data.generatedAt).toLocaleTimeString()}`;
  }

  async function load({ reload = false } = {}) {
    if (busy || (loaded && !reload)) return;
    busy = true;
    byId("health-status").textContent = "Loading";
    try {
      const hours = byId("health-window").value;
      render(await request(`/api/metrics?hours=${encodeURIComponent(hours)}`));
      loaded = true;
    } catch (error) {
      showError(error);
      byId("health-status").textContent = "Load failed";
    } finally {
      busy = false;
    }
  }

  return { load };
}
