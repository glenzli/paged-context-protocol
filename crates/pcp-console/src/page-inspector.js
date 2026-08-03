import { createTopologyMap, relationFamily } from "./page-graph.js";
import { renderPageContent } from "./page-content.js";

export function createPageInspector({ request, showError, formatTime }) {
  const dialog = document.getElementById("page-dialog");
  const backButton = document.getElementById("dialog-back");
  const title = document.getElementById("dialog-title");
  const subtitle = document.getElementById("dialog-subtitle");
  const summaryPane = document.getElementById("detail-summary");
  const graphPane = document.getElementById("detail-graph");
  const historyPane = document.getElementById("detail-history");
  const rawPane = document.getElementById("detail-raw");
  const detailCache = new Map();
  const graphCache = new Map();
  const historyCache = new Map();
  const rawCache = new Map();
  const navigationHistory = [];
  let currentPageId = null;
  let currentView = "summary";
  let currentGraphFilter = "all";
  let currentGraphDepth = 2;
  let currentGraphLimit = 120;

  const relationFamilies = [
    ["all", "All relations"],
    ["derivation", "Derivation"],
    ["conversation", "Conversation"],
    ["evidence", "Evidence"],
    ["semantic", "Semantic"],
  ];

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = String(text);
    return node;
  }

  function detailSection(label, content) {
    const section = element("section", "detail-section");
    section.append(element("h3", "", label), content);
    return section;
  }

  function jsonBlock(value, fallback) {
    return element("pre", "", value ? JSON.stringify(value, null, 2) : fallback);
  }

  function contentBlock(content, mediaType = "text/markdown") {
    const block = element("div", "page-content");
    renderPageContent(block, content, mediaType);
    return block;
  }

  function actorLabel(actor) {
    if (!actor) return "-";
    const prefix = `${actor.actorType}:`;
    return actor.actorId.startsWith(prefix) ? actor.actorId : `${prefix}${actor.actorId}`;
  }

  function pageLabel(page) {
    const facetTitle = page.page.facets?.title;
    if (facetTitle) return facetTitle;
    const firstLine = page.summary?.content?.split("\n").find((line) => line.trim());
    return firstLine?.replace(/^#+\s*/, "").slice(0, 120) || page.page.pageId;
  }

  function truncate(value, limit) {
    if (!value) return "";
    const compact = value.replace(/\s+/g, " ").trim();
    return compact.length > limit ? `${compact.slice(0, limit)}…` : compact;
  }

  function graphPreview(label, value) {
    const node = element("span", "graph-preview");
    node.append(element("span", "graph-preview-label", label), element("span", "", value));
    return node;
  }

  function renderSummary(page) {
    const preview = page.summary?.content || page.page.payload?.content || "No summary projection";
    const previewMediaType = page.summary
      ? "text/markdown"
      : page.page.payload?.mediaType || "text/plain";
    const facts = element("dl", "details-grid compact-details");
    const rows = [
      ["Scope", page.page.namespace],
      ["Status", page.page.lifecycleStatus],
      ["Observed", formatTime(page.page.observedAt || page.page.createdAt)],
      ["Created by", actorLabel(page.page.createdBy)],
      ["Relations", page.relations.length],
      ["Lineage", page.lineage.length],
    ];
    facts.append(...rows.flatMap(([label, value]) => [
      element("dt", "", label),
      element("dd", label === "Scope" || label === "Created by" ? "mono" : "", value),
    ]));

    const sections = [
      detailSection(page.summary ? "Summary" : "Preview", contentBlock(preview, previewMediaType)),
      detailSection("Page", facts),
    ];
    if (page.validity) sections.push(detailSection("Validity", jsonBlock(page.validity, "No validity assessment")));
    summaryPane.replaceChildren(...sections);
  }

  function graphNode(page, relations, direction, detailPreview) {
    const button = element("button", "graph-node");
    button.type = "button";
    button.title = `Open ${page.page.pageId}`;
    const relationTypes = [...new Set(relations.map((relation) => relation.relationType))];
    const family = relationFamily(relationTypes[0]);
    button.append(
      element("span", `graph-relation relation-${family}`, `${direction} · ${relationTypes.join(" / ")}`),
      element("strong", "", pageLabel(page)),
      element("span", "mono muted", page.page.pageId),
      element("span", "mono muted", page.page.namespace),
    );
    const summaryPreview = truncate(page.summary?.content, 220);
    const payloadPreview = truncate(detailPreview, 280);
    if (summaryPreview) button.append(graphPreview("Summary", summaryPreview));
    if (payloadPreview) button.append(graphPreview("Detail", payloadPreview));
    button.addEventListener("click", () => navigate(page.page.pageId));
    return button;
  }

  function graphFilters(relations, graph) {
    const controls = element("div", "graph-filters");
    for (const [value, label] of relationFamilies) {
      const count = value === "all"
        ? relations.length
        : relations.filter((relation) => relationFamily(relation.relationType) === value).length;
      const button = element("button", `graph-filter${currentGraphFilter === value ? " active" : ""}`, `${label} ${count}`);
      button.type = "button";
      button.dataset.relationFamily = value;
      button.disabled = count === 0;
      button.addEventListener("click", () => {
        currentGraphFilter = value;
        renderGraph(graph);
      });
      controls.append(button);
    }
    return controls;
  }

  function graphQueryControls() {
    const controls = element("div", "graph-query-controls");
    const depthControl = element("div", "graph-depth-control");
    depthControl.append(element("span", "muted", "Depth"));
    for (const depth of [1, 2, 3]) {
      const button = element("button", `graph-depth-button${currentGraphDepth === depth ? " active" : ""}`, `${depth} hop${depth > 1 ? "s" : ""}`);
      button.type = "button";
      button.addEventListener("click", () => {
        if (currentGraphDepth === depth) return;
        currentGraphDepth = depth;
        loadGraph(currentPageId);
      });
      depthControl.append(button);
    }
    const budgetLabel = element("label", "graph-budget-control");
    budgetLabel.append(element("span", "muted", "Node budget"));
    const budget = element("select", "");
    for (const value of [60, 120, 240]) {
      const option = element("option", "", value);
      option.value = String(value);
      option.selected = currentGraphLimit === value;
      budget.append(option);
    }
    budget.addEventListener("change", () => {
      currentGraphLimit = Number(budget.value);
      loadGraph(currentPageId);
    });
    budgetLabel.append(budget);
    controls.append(depthControl, budgetLabel);
    return controls;
  }

  function renderGraph(graph) {
    const root = graph.root;
    const rootId = root.page.pageId;
    const neighbors = new Map(graph.neighbors.map((page) => [page.page.pageId, page]));
    const snippets = new Map(graph.hits.map((hit) => [hit.pageId, hit.snippet]));
    const incoming = new Map();
    const outgoing = new Map();
    const visibleRelations = currentGraphFilter === "all"
      ? root.relations
      : root.relations.filter((relation) => relationFamily(relation.relationType) === currentGraphFilter);
    for (const relation of visibleRelations) {
      const incomingEdge = relation.toPageId === rootId;
      const neighborId = incomingEdge ? relation.fromPageId : relation.toPageId;
      const neighbor = neighbors.get(neighborId);
      if (!neighbor) continue;
      const lane = incomingEdge ? incoming : outgoing;
      const entry = lane.get(neighborId) || { page: neighbor, relations: [] };
      entry.relations.push(relation);
      lane.set(neighborId, entry);
    }

    const rootNode = element("div", "graph-root");
    rootNode.append(
      element("span", "graph-relation", "current"),
      element("strong", "", pageLabel(root)),
      element("span", "mono muted", rootId),
    );
    const lanes = element("div", "graph-lanes");
    for (const [label, entries] of [["Incoming", incoming], ["Outgoing", outgoing]]) {
      const nodes = [...entries.values()].map((entry) => graphNode(
        entry.page,
        entry.relations,
        label.toLowerCase(),
        snippets.get(entry.page.page.pageId),
      ));
      const lane = element("section", "graph-lane");
      lane.append(element("h3", "", `${label} · ${nodes.length}`));
      if (nodes.length) lane.append(...nodes);
      else lane.append(element("div", "empty graph-empty", "No relations"));
      lanes.append(lane);
    }
    const relationHeading = element("h3", "graph-section-title", "Direct relations");
    graphPane.replaceChildren(
      graphQueryControls(),
      graphFilters(graph.topology.edges, graph),
      createTopologyMap({
        topology: graph.topology,
        relationFilter: currentGraphFilter,
        onNavigate: (pageId) => navigate(pageId),
      }),
      relationHeading,
      rootNode,
      lanes,
    );
  }

  function renderHistory(lineage) {
    const list = element("div", "history-list");
    lineage.pages.forEach((page, index) => {
      const button = element("button", `history-entry${page.page.pageId === currentPageId ? " selected" : ""}`);
      button.type = "button";
      const heading = element("span", "history-heading");
      heading.append(
        element("strong", "mono", page.page.pageId),
        element("span", "muted", index === 0 ? "Current" : formatTime(page.page.createdAt)),
      );
      const standing = page.validity?.standing ? ` · ${page.validity.standing}` : "";
      button.append(
        heading,
        element("span", "muted", `${page.page.lifecycleStatus}${standing}`),
        element("span", "history-preview", truncate(page.summary?.content || page.page.payload?.content, 360) || "No content projection"),
      );
      button.addEventListener("click", () => {
        if (page.page.pageId !== currentPageId) navigate(page.page.pageId);
      });
      list.append(button);
    });
    const status = lineage.total > lineage.pages.length
      ? `${lineage.pages.length} of ${lineage.total} Pages in this lineage`
      : `${lineage.pages.length} Pages in this lineage`;
    historyPane.replaceChildren(element("div", "history-status muted", status), list);
  }

  function renderRaw(page) {
    const payload = page.page.payload;
    const manifest = {
      lifecycleStatus: page.page.lifecycleStatus,
      createdAt: page.page.createdAt,
      observedAt: page.page.observedAt,
      validFrom: page.page.validFrom,
      validTo: page.page.validTo,
      createdBy: page.page.createdBy,
      facets: page.page.facets,
    };
    rawPane.replaceChildren(
      detailSection(
        payload?.mediaType ? `Detail · ${payload.mediaType}` : "Detail",
        contentBlock(payload?.content || "No payload projection", payload?.mediaType || "text/plain"),
      ),
      detailSection("Manifest", jsonBlock(manifest, "No manifest")),
      detailSection("Sources and provenance", jsonBlock({
        sourceRefs: page.page.sourceRefs,
        provenance: page.page.provenance,
      }, "No sources or provenance")),
    );
  }

  async function loadGraph(pageId) {
    graphPane.replaceChildren(element("div", "loading", "Loading graph"));
    try {
      const cacheKey = `${pageId}:${currentGraphDepth}:${currentGraphLimit}`;
      let graph = graphCache.get(cacheKey);
      if (!graph) {
        graph = await request(`/api/pages/${encodeURIComponent(pageId)}/graph?depth=${currentGraphDepth}&limit=${currentGraphLimit}`);
        graphCache.set(cacheKey, graph);
      }
      if (currentPageId === pageId) renderGraph(graph);
    } catch (error) {
      showError(error);
      graphPane.replaceChildren(element("div", "empty", "Graph unavailable"));
    }
  }

  async function loadRaw(pageId) {
    rawPane.replaceChildren(element("div", "loading", "Loading detail"));
    try {
      let page = rawCache.get(pageId);
      if (!page) {
        page = await request(`/api/pages/${encodeURIComponent(pageId)}/raw`);
        rawCache.set(pageId, page);
      }
      if (currentPageId === pageId) renderRaw(page);
    } catch (error) {
      showError(error);
      rawPane.replaceChildren(element("div", "empty", "Detail unavailable"));
    }
  }

  async function loadHistory(pageId) {
    historyPane.replaceChildren(element("div", "loading", "Loading lineage"));
    try {
      let lineage = historyCache.get(pageId);
      if (!lineage) {
        lineage = await request(`/api/pages/${encodeURIComponent(pageId)}/lineage`);
        historyCache.set(pageId, lineage);
      }
      if (currentPageId === pageId) renderHistory(lineage);
    } catch (error) {
      showError(error);
      historyPane.replaceChildren(element("div", "empty", "Lineage unavailable"));
    }
  }

  function activate(view) {
    currentView = view;
    document.querySelectorAll(".detail-tab").forEach((tab) => tab.classList.toggle("active", tab.dataset.detailView === view));
    document.querySelectorAll(".detail-view").forEach((pane) => pane.classList.toggle("active", pane.id === `detail-${view}`));
    if (view === "graph" && currentPageId) loadGraph(currentPageId);
    if (view === "history" && currentPageId) loadHistory(currentPageId);
    if (view === "raw" && currentPageId) loadRaw(currentPageId);
  }

  async function inspect(pageId, restoredView = "summary") {
    currentPageId = pageId;
    backButton.hidden = navigationHistory.length === 0;
    activate("summary");
    title.textContent = "Loading page";
    subtitle.textContent = pageId;
    summaryPane.replaceChildren(element("div", "loading", "Loading summary"));
    graphPane.replaceChildren();
    historyPane.replaceChildren();
    rawPane.replaceChildren();
    try {
      let page = detailCache.get(pageId);
      if (!page) {
        page = await request(`/api/pages/${encodeURIComponent(pageId)}`);
        detailCache.set(pageId, page);
      }
      if (currentPageId !== pageId) return;
      title.textContent = page.page.pageId;
      subtitle.textContent = page.page.refId
        ? `${page.page.namespace} · Ref ${page.page.refId}`
        : page.page.namespace;
      renderSummary(page);
      if (!dialog.open) dialog.showModal();
      dialog.scrollTop = 0;
      if (restoredView !== "summary") activate(restoredView);
    } catch (error) { showError(error); }
  }

  function open(pageId) {
    navigationHistory.length = 0;
    return inspect(pageId);
  }

  function navigate(pageId) {
    if (currentPageId) navigationHistory.push({ pageId: currentPageId, view: currentView });
    return inspect(pageId);
  }

  function back() {
    const entry = navigationHistory.pop();
    if (entry) return inspect(entry.pageId, entry.view);
    return Promise.resolve();
  }

  document.querySelectorAll(".detail-tab").forEach((tab) => {
    tab.addEventListener("click", () => activate(tab.dataset.detailView));
  });
  backButton.addEventListener("click", () => back());
  document.getElementById("dialog-close").addEventListener("click", () => {
    navigationHistory.length = 0;
    dialog.close();
  });

  return { open };
}
