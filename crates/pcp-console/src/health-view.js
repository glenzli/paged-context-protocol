export function createHealthView({ request, showError, formatNumber, t }) {
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

  function percentage(part, total) {
    return total ? `${Math.round((part / total) * 100)}%` : "-";
  }

  function decimalRatio(part, total) {
    return total ? (part / total).toFixed(1) : "-";
  }

  function hourlyRate(value, hours) {
    return hours ? (value / hours).toFixed(value / hours < 10 ? 1 : 0) : "-";
  }

  function duration(value) {
    if (value === null || value === undefined) return t("Collecting");
    if (value < 1000) return `${formatNumber(value)} ms`;
    return `${(value / 1000).toFixed(2)} s`;
  }

  function outputSize(bytes) {
    if (bytes < 1000) return `${formatNumber(bytes)} B`;
    if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB`;
    return `${(bytes / 1_000_000).toFixed(2)} MB`;
  }

  function panel(label, description, entries) {
    const node = element("section", "aggregate-panel");
    const heading = element("div", "aggregate-panel-heading");
    heading.append(element("h3", "", label));
    if (description) heading.append(element("p", "", description));
    node.append(heading);
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
      signals.push(["info", t("No workload calls were observed in this window. Telemetry begins with the upgraded runtime.")]);
    }
    if (data.activity.calls && data.activity.measuredCalls < data.activity.calls) {
      signals.push(["info", `${percentage(data.activity.measuredCalls, data.activity.calls)} ${t("of calls in this window include detailed telemetry.")}`]);
    }
    if (data.recall.searches) {
      const zeroRate = data.recall.zeroResultSearches / data.recall.searches;
      if (zeroRate >= 0.35) signals.push(["warning", `${percentage(data.recall.zeroResultSearches, data.recall.searches)} ${t("of measured searches returned no Page.")}`]);
      else signals.push(["positive", `${percentage(data.recall.searches - data.recall.zeroResultSearches, data.recall.searches)} ${t("of measured searches returned at least one Page. This does not establish relevance.")}`]);
      const recallCalls = data.recall.searches + data.recall.summaryReads + data.recall.detailReads;
      const callsPerHour = recallCalls / data.windowHours;
      const readsPerSearch = (data.recall.summaryReads + data.recall.detailReads) / data.recall.searches;
      if (callsPerHour >= 120 && readsPerSearch >= 2) {
        signals.push(["warning", `${t("Recall is issuing")} ${hourlyRate(recallCalls, data.windowHours)} ${t("search/read calls per hour at")} ${readsPerSearch.toFixed(1)} ${t("read calls per search. Check Access if the client should be idle.")}`]);
      }
    }
    if (data.activity.failed || data.activity.denied) {
      signals.push(["danger", `${formatNumber(data.activity.failed)} ${t("failed and")} ${formatNumber(data.activity.denied)} ${t("denied calls need inspection.")}`]);
    }
    const target = byId("health-signals");
    target.hidden = !signals.length;
    target.replaceChildren(...signals.map(([tone, text]) => element("div", `health-signal tone-${tone}`, text)));
  }

  function renderTimeline(data) {
    const buckets = data.timeline || [];
    if (!buckets.length) {
      byId("health-timeline").replaceChildren(element("div", "empty", t("No workload activity in this window")));
      return;
    }
    const maximum = Math.max(...buckets.map((bucket) => bucket.calls), 1);
    const chart = element("div", "timeline-chart");
    for (const bucket of buckets) {
      const item = element("div", "timeline-bucket");
      const bars = element("div", "timeline-bars");
      const calls = element("div", "timeline-bar calls");
      calls.style.height = `${Math.max(3, Math.round((bucket.calls / maximum) * 72))}px`;
      calls.title = `${bucket.calls} ${t("calls")}`;
      const failures = element("div", "timeline-bar failures");
      failures.style.height = `${Math.round((bucket.failures / maximum) * 72)}px`;
      failures.title = `${bucket.failures} ${t("failures")}`;
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
            : `${formatNumber(operation.calls)} · ${formatNumber(operation.measuredCalls)} ${t("measured")}`),
          element("td", operation.failures ? "health-danger" : "", formatNumber(operation.failures)),
          element("td", "", `${formatNumber(operation.outputCount)} · ${outputSize(operation.outputBytes)}`),
          element("td", "", duration(operation.p50DurationMs)),
          element("td", "", duration(operation.p95DurationMs)),
        );
        return row;
      });
    if (!rows.length) {
      const row = document.createElement("tr");
      const cell = element("td", "empty", t("No workload operations in this window"));
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

  function modelUsageLabel(source) {
    if (source === "query") return t("Intent matching");
    if (source === "manual_maintenance") return t("Manual maintenance");
    if (source === "automatic_maintenance") return t("Automatic maintenance");
    return source;
  }

  function renderModelUsage(data) {
    const usage = data.modelUsage || {};
    const totalCalls = usage.modelCalls || 0;
    const reportedCalls = usage.reportedModelCalls || 0;
    const totalTokens = usage.usage?.totalTokens || 0;
    const sources = usage.sources || [];
    const entries = sources.length
      ? sources.map((source) => [
          `${modelUsageLabel(source.source)} · ${source.operation}`,
          `${formatNumber(source.usage?.totalTokens || 0)} ${t("tokens")} · ${formatNumber(source.reportedModelCalls || 0)}/${formatNumber(source.modelCalls || 0)}`,
        ])
      : [[t("No model usage was observed in this window"), "-"]];
    byId("health-model-usage").replaceChildren(
      panel(t("Model usage"), "", [
        [t("Model calls"), formatNumber(totalCalls)],
        [t("Reported tokens"), formatNumber(totalTokens)],
        [t("Token reporting"), `${formatNumber(reportedCalls)}/${formatNumber(totalCalls)} · ${percentage(reportedCalls, totalCalls)}`],
      ]),
      panel(t("By workflow"), "", entries),
    );
  }

  function render(data) {
    latest = data;
    const failureCount = data.activity.failed + data.activity.denied;
    byId("health-metrics").replaceChildren(
      metric(t("Observed calls"), formatNumber(data.activity.calls), "", t("Authorized client operations in this window")),
      metric(t("Failed / denied"), `${formatNumber(data.activity.failed)} / ${formatNumber(data.activity.denied)}`, failureCount ? "danger" : "positive", t("Runtime failures and authorization denials")),
      metric(t("p95 latency"), duration(data.activity.p95DurationMs), data.activity.p95DurationMs === null || data.activity.p95DurationMs === undefined ? "" : "info", t("Measured response latency")),
      metric(t("Telemetry coverage"), percentage(data.activity.measuredCalls, data.activity.calls), data.activity.measuredCalls < data.activity.calls ? "warning" : "positive", `${formatNumber(data.activity.measuredCalls)} ${t("measured calls")}`),
    );
    byId("health-flows").replaceChildren(
      panel(t("Recall activity"), "", [
        [t("Searches"), `${formatNumber(data.recall.searches)} · ${hourlyRate(data.recall.searches, data.windowHours)}/h`],
        [t("Zero-result"), percentage(data.recall.zeroResultSearches, data.recall.searches), data.recall.searches && data.recall.zeroResultSearches / data.recall.searches >= 0.35 ? "warning" : "positive"],
        [t("Pages returned"), `${formatNumber(data.recall.returnedPages)} · ${decimalRatio(data.recall.returnedPages, data.recall.searches)}/${t("search")}`],
        [t("Summary / detail reads"), `${formatNumber(data.recall.summaryReads)} / ${formatNumber(data.recall.detailReads)} · ${decimalRatio(data.recall.summaryReads + data.recall.detailReads, data.recall.searches)}/${t("search")}`],
      ]),
      panel(t("Runtime service"), "", [
        [t("Principals"), formatNumber(data.activity.principals)],
        [t("Telemetry coverage"), percentage(data.activity.measuredCalls, data.activity.calls)],
        [t("p50 latency"), duration(data.activity.p50DurationMs)],
        [t("p95 latency"), duration(data.activity.p95DurationMs)],
      ]),
    );
    renderModelUsage(data);
    renderSignals(data);
    renderTimeline(data);
    renderOperations(data);
    renderScopes(data);
    byId("health-status").textContent = `${data.windowHours}h ${t("window")} · ${t("Updated")} ${new Date(data.generatedAt).toLocaleTimeString()}`;
  }

  async function load({ reload = false } = {}) {
    if (busy || (loaded && !reload)) return;
    busy = true;
    byId("health-status").textContent = t("Loading");
    try {
      const hours = byId("health-window").value;
      render(await request(`/api/metrics?hours=${encodeURIComponent(hours)}`));
      loaded = true;
    } catch (error) {
      showError(error);
      byId("health-status").textContent = t("Load failed");
    } finally {
      busy = false;
    }
  }

  function rerender() {
    if (latest) render(latest);
  }

  return { load, rerender };
}
