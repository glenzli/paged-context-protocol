import { createTopologyMap, relationFamily } from "./page-graph.js";
import { pagePayloadPreviewText, renderPageContent, renderPagePreview } from "./page-content.js?v=20260822.1";

export function createPageInspector({ request, showError, formatTime, t = (value) => value }) {
  const dialog = document.getElementById("page-dialog");
  const backButton = document.getElementById("dialog-back");
  const title = document.getElementById("dialog-title");
  const subtitle = document.getElementById("dialog-subtitle");
  const summaryPane = document.getElementById("detail-summary");
  const graphPane = document.getElementById("detail-graph");
  const historyPane = document.getElementById("detail-history");
  const rawPane = document.getElementById("detail-raw");
  const relationComparisonDialog = document.getElementById("relation-comparison-dialog");
  const relationComparisonSubtitle = document.getElementById("relation-comparison-subtitle");
  const relationComparisonReason = document.getElementById("relation-comparison-reason");
  const relationComparisonReviewNote = document.getElementById("relation-comparison-review-note");
  const relationComparisonPages = document.getElementById("relation-comparison-pages");
  const relationComparisonAccept = document.getElementById("relation-comparison-accept");
  const relationComparisonReject = document.getElementById("relation-comparison-reject");
  const relationComparisonSkip = document.getElementById("relation-comparison-skip");
  const relationComparisonDecisionGroup = relationComparisonAccept.closest(".relation-comparison-decision-group");
  const topicExtractionDialog = document.getElementById("topic-extraction-review-dialog");
  const topicExtractionSubtitle = document.getElementById("topic-extraction-review-subtitle");
  const topicExtractionReason = document.getElementById("topic-extraction-review-reason");
  const topicExtractionTitle = document.getElementById("topic-extraction-review-title");
  const topicExtractionProposal = document.getElementById("topic-extraction-review-proposal");
  const topicExtractionTabs = document.getElementById("topic-extraction-review-tabs");
  const topicExtractionPage = document.getElementById("topic-extraction-review-page");
  const detailCache = new Map();
  const graphCache = new Map();
  const historyCache = new Map();
  const rawCache = new Map();
  const reviewedRevisionCache = new Map();
  const navigationHistory = [];
  let currentPageId = null;
  let currentView = "summary";
  let currentGraphFilter = "all";
  let currentGraphDepth = 2;
  let currentGraphLimit = 120;
  let comparisonAcceptAction = null;
  let comparisonRejectAction = null;
  let comparisonSkipAction = null;
  let comparisonScrollTop = 0;

  const relationFamilies = [
    ["all", "All connections"],
    ["source_stream", "Source stream"],
    ["derivation", "Derivation"],
    ["provenance", "Provenance inputs"],
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

  function rawContentBlock(content, mediaType = "text/plain") {
    const normalizedMediaType = mediaType.split(";", 1)[0].trim().toLowerCase();
    const isJson = normalizedMediaType === "application/json" || normalizedMediaType.endsWith("+json");
    if (isJson && typeof content === "string") {
      try {
        return element("pre", "", JSON.stringify(JSON.parse(content), null, 2));
      } catch {
        // Keep malformed JSON available exactly as stored.
      }
    }
    return contentBlock(content, mediaType);
  }

  function previewBlock(content, mediaType, options = {}) {
    const block = element("div", "page-content");
    renderPagePreview(block, content, mediaType, {
      ...options,
      packedEntryControls: {
        expandAll: t("Expand all entries"),
        collapseAll: t("Collapse all entries"),
      },
    });
    return block;
  }

  function actorLabel(actor) {
    if (!actor) return "-";
    const prefix = `${actor.actorType}:`;
    return actor.actorId.startsWith(prefix) ? actor.actorId : `${prefix}${actor.actorId}`;
  }

  function pageLabel(page) {
    const facetTitle = page.revision.facets?.title;
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
    node.append(element("span", "graph-preview-label", t(label)), element("span", "", value));
    return node;
  }

  function renderSummary(page) {
    const payload = page.revision.payload;
    const summaryTarget = page.relations.find((relation) => (
      relation.relationType === "summarizes" && relation.fromPageId === page.page.pageId
    ))?.toPageId;
    const facts = element("dl", "details-grid compact-details");
    const rows = [
      ["Scope", page.page.namespace],
      ["Kind", page.page.kind],
      ["Mutability", page.page.mutability],
      ["Status", page.page.lifecycleStatus],
      ["Revision", page.revision.revisionId],
      ["Observed", formatTime(page.revision.observedAt || page.revision.createdAt)],
      ["Created by", actorLabel(page.revision.createdBy)],
      ...(page.revision.sourceSpan ? [[
        "Source stream",
        `${page.revision.sourceSpan.streamId} · ${page.revision.sourceSpan.start}–${page.revision.sourceSpan.end}`,
      ]] : []),
      ...(page.summary ? [["Summary page", page.summary.summaryPageId]] : []),
      ...(summaryTarget ? [["Summarizes", summaryTarget]] : []),
      ["Explicit relations", page.relations.length],
      ["History", page.history.length],
    ];
    facts.append(...rows.flatMap(([label, value]) => [
      element("dt", "", t(label)),
      element("dd", label === "Scope" || label === "Created by" ? "mono" : "", value),
    ]));

    const sections = [];
    if (page.summary) {
      sections.push(detailSection(
        t("Summary"),
        contentBlock(page.summary.content, "text/markdown"),
      ));
    }
    sections.push(detailSection(
      t(page.page.kind === "summary_projection" ? "Summary content" : "Content"),
      previewBlock(
        payload?.content || t("No content projection"),
        payload?.mediaType || "text/plain",
      ),
    ));
    sections.push(detailSection(t("Page"), facts));
    if (page.validity) sections.push(detailSection(t("Validity"), jsonBlock(page.validity, t("No validity assessment"))));
    summaryPane.replaceChildren(...sections);
  }

  function graphNode(page, relations, direction, detailPreview) {
    const button = element("button", "graph-node");
    button.type = "button";
    button.title = `Open ${page.page.pageId}`;
    const relationTypes = [...new Set(relations.map((relation) => relation.edgeKind === "source_stream"
      ? "contiguous source span"
      : relation.edgeKind === "provenance"
        ? `provenance input · ${relation.relationType}`
        : relation.relationType))];
    const family = relationFamily(relations[0].relationType, relations[0].edgeKind);
    button.append(
      element("span", `graph-relation relation-${family}`, `${direction} · ${relationTypes.join(" / ")}`),
      element("strong", "", pageLabel(page)),
      element("span", "mono muted", page.page.pageId),
      element("span", "mono muted", page.revision.revisionId),
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
        : relations.filter((relation) => relationFamily(relation.relationType, relation.edgeKind) === value).length;
      const button = element("button", `graph-filter${currentGraphFilter === value ? " active" : ""}`, `${t(label)} ${count}`);
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
    depthControl.append(element("span", "muted", t("Traversal depth")));
    for (const depth of [1, 2, 3]) {
      const button = element("button", `graph-depth-button${currentGraphDepth === depth ? " active" : ""}`, `${depth} ${t(depth > 1 ? "hops" : "hop")}`);
      button.type = "button";
      button.title = `Show Pages within ${depth} graph edge${depth > 1 ? "s" : ""} of the current Page`;
      button.setAttribute("aria-label", button.title);
      button.addEventListener("click", () => {
        if (currentGraphDepth === depth) return;
        currentGraphDepth = depth;
        loadGraph(currentPageId);
      });
      depthControl.append(button);
    }
    const budgetLabel = element("label", "graph-budget-control");
    budgetLabel.append(element("span", "muted", t("Node budget")));
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
    const directEdges = graph.topology.edges.filter((edge) => edge.fromPageId === rootId || edge.toPageId === rootId);
    const visibleRelations = currentGraphFilter === "all"
      ? directEdges
      : directEdges.filter((edge) => relationFamily(edge.relationType, edge.edgeKind) === currentGraphFilter);
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
        snippets.get(entry.page.page.pageId)
          || pagePayloadPreviewText(
            entry.page.revision.payload?.content,
            entry.page.revision.payload?.mediaType,
          ),
      ));
      const lane = element("section", "graph-lane");
      lane.append(element("h3", "", `${label} · ${nodes.length}`));
      if (nodes.length) lane.append(...nodes);
      else lane.append(element("div", "empty graph-empty", "No connections"));
      lanes.append(lane);
    }
    const relationHeading = element("h3", "graph-section-title", "Direct connections");
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
      const selected = page.revision.revisionId === currentPageId
        || (page.page.pageId === currentPageId && page.revision.revisionId === page.page.headRevisionId);
      const button = element("button", `history-entry${selected ? " selected" : ""}`);
      button.type = "button";
      const heading = element("span", "history-heading");
      heading.append(
        element("strong", "mono", page.revision.revisionId),
        element("span", "muted", index === 0 ? "Current head" : formatTime(page.revision.createdAt)),
      );
      const standing = page.validity?.standing ? ` · ${page.validity.standing}` : "";
      button.append(
        heading,
        element("span", "muted", `${page.page.lifecycleStatus} · ${page.page.mutability}${standing}`),
        element(
          "span",
          "history-preview",
          truncate(
            page.summary?.content
              || pagePayloadPreviewText(page.revision.payload?.content, page.revision.payload?.mediaType),
            360,
          ) || "No content projection",
        ),
      );
      button.addEventListener("click", () => {
        if (page.revision.revisionId !== currentPageId) navigate(page.revision.revisionId);
      });
      list.append(button);
    });
    const status = lineage.total > lineage.pages.length
      ? `${lineage.pages.length} of ${lineage.total} Revisions`
      : `${lineage.pages.length} Revisions`;
    historyPane.replaceChildren(element("div", "history-status muted", status), list);
  }

  function renderRaw(page) {
    const payload = page.revision.payload;
    const manifest = {
      page: page.page,
      revision: {
        pageId: page.revision.pageId,
        revisionId: page.revision.revisionId,
        previousRevisionId: page.revision.previousRevisionId,
        lifecycleStatus: page.revision.lifecycleStatus,
        createdAt: page.revision.createdAt,
        observedAt: page.revision.observedAt,
        validFrom: page.revision.validFrom,
        validTo: page.revision.validTo,
        createdBy: page.revision.createdBy,
        facets: page.revision.facets,
      },
    };
    rawPane.replaceChildren(
      detailSection(
        payload?.mediaType ? `Raw content · ${payload.mediaType}` : "Raw content",
        rawContentBlock(payload?.content || "No payload projection", payload?.mediaType || "text/plain"),
      ),
      detailSection("Manifest", jsonBlock(manifest, "No manifest")),
      detailSection("Sources and provenance", jsonBlock({
        sourceRefs: page.revision.sourceRefs,
        provenance: page.revision.provenance,
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
      rawPane.replaceChildren(element("div", "empty", "Raw content unavailable"));
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
      historyPane.replaceChildren(element("div", "empty", "History unavailable"));
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
      subtitle.textContent = `${page.page.namespace} · ${page.page.kind} · ${page.page.mutability} · ${page.revision.revisionId}`;
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

  function comparisonPage(page, position) {
    const section = element("section", "relation-comparison-page");
    const heading = element("header", "relation-comparison-page-heading");
    heading.append(
      element("span", "relation-comparison-page-position", t(position)),
      element("strong", "mono", page.pageId),
      element("span", "mono muted", page.revisionId),
    );
    const body = element("div", "relation-comparison-page-body");
    body.append(element("div", "loading", t("Loading full Page…")));
    section.append(heading, body);
    return { section, body };
  }

  async function reviewedRevision(page) {
    const cached = reviewedRevisionCache.get(page.revisionId);
    if (cached) return cached;
    const detail = await request(`/api/pages/${encodeURIComponent(page.revisionId)}`);
    if (detail.revision?.revisionId !== page.revisionId) {
      throw new Error(t("The reviewed revision is no longer available."));
    }
    const payload = detail.revision?.payload;
    const value = {
      content: payload?.content || t("No content projection"),
      mediaType: payload?.mediaType || "text/plain",
    };
    reviewedRevisionCache.set(page.revisionId, value);
    return value;
  }

  async function loadComparedRevision(page, target, isCurrent = () => true) {
    try {
      const payload = await reviewedRevision(page);
      if (!isCurrent()) return;
      target.replaceChildren(previewBlock(
        payload.content,
        payload.mediaType,
      ));
    } catch (error) {
      if (!isCurrent()) return;
      target.replaceChildren(element("div", "empty", error.message || String(error)));
      showError(error);
    }
  }

  function lockRelationComparisonScroll() {
    comparisonScrollTop = window.scrollY;
    document.body.style.setProperty("--relation-comparison-scroll-offset", `-${comparisonScrollTop}px`);
    document.documentElement.classList.add("relation-comparison-open");
  }

  function unlockRelationComparisonScroll() {
    document.documentElement.classList.remove("relation-comparison-open");
    document.body.style.removeProperty("--relation-comparison-scroll-offset");
    window.scrollTo(0, comparisonScrollTop);
  }

  function compareRelation({
    pages,
    relationReason,
    reviewReason,
    onAccept = null,
    onReject = null,
    onSkip = null,
    accepted = false,
    rejected = false,
    skipped = false,
  }) {
    if (!Array.isArray(pages) || pages.length !== 2) {
      const error = new Error(t("A relation comparison requires exactly two Pages."));
      showError(error);
      return Promise.resolve();
    }
    relationComparisonSubtitle.textContent = `${t("Explicit relation")} · related_to`;
    relationComparisonReason.textContent = relationReason || t("No relation rationale was supplied.");
    relationComparisonReviewNote.hidden = !reviewReason;
    relationComparisonReviewNote.textContent = reviewReason || "";
    comparisonAcceptAction = typeof onAccept === "function" ? onAccept : null;
    comparisonRejectAction = typeof onReject === "function" ? onReject : null;
    comparisonSkipAction = typeof onSkip === "function" ? onSkip : null;
    relationComparisonDecisionGroup.hidden = !comparisonAcceptAction && !comparisonRejectAction && !comparisonSkipAction;
    relationComparisonAccept.hidden = !comparisonAcceptAction;
    relationComparisonAccept.classList.toggle("is-accepted", accepted);
    relationComparisonAccept.setAttribute("aria-pressed", String(accepted));
    relationComparisonAccept.title = t(accepted ? "Accepted" : "Accept");
    relationComparisonAccept.setAttribute("aria-label", relationComparisonAccept.title);
    relationComparisonReject.hidden = !comparisonRejectAction;
    relationComparisonReject.classList.toggle("is-rejected", rejected);
    relationComparisonReject.setAttribute("aria-pressed", String(rejected));
    relationComparisonReject.title = t(rejected ? "Rejected for this review" : "Reject");
    relationComparisonReject.setAttribute("aria-label", relationComparisonReject.title);
    relationComparisonSkip.hidden = !comparisonSkipAction;
    relationComparisonSkip.classList.toggle("is-skipped", skipped);
    relationComparisonSkip.setAttribute("aria-pressed", String(skipped));
    relationComparisonSkip.title = t(skipped ? "Skipped for now" : "Skip for now");
    relationComparisonSkip.setAttribute("aria-label", relationComparisonSkip.title);
    const left = comparisonPage(pages[0], "Left Page");
    const right = comparisonPage(pages[1], "Right Page");
    relationComparisonPages.replaceChildren(left.section, right.section);
    if (!relationComparisonDialog.open) {
      lockRelationComparisonScroll();
      relationComparisonDialog.showModal();
    }
    relationComparisonDialog.scrollTop = 0;
    return Promise.all([
      loadComparedRevision(pages[0], left.body),
      loadComparedRevision(pages[1], right.body),
    ]);
  }

  function topicSourceTab(page, index, active, select) {
    const tab = element("button", `topic-extraction-review-tab${active ? " active" : ""}`);
    tab.type = "button";
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(active));
    tab.title = `${page.pageId} · ${page.revisionId}`;
    tab.append(
      element("span", "topic-extraction-review-tab-index", `${index + 1}`),
      element("span", "mono", page.pageId),
    );
    tab.addEventListener("click", () => select(index));
    return tab;
  }

  function reviewTopic(candidate) {
    const pages = candidate?.pages;
    if (!Array.isArray(pages) || pages.length < 2) {
      const error = new Error(t("A Topic extraction review requires at least two Pages."));
      showError(error);
      return Promise.resolve();
    }
    topicExtractionSubtitle.textContent = candidate.refreshTarget
      ? `${candidate.namespace} · ${pages.length} ${t("Source Pages")} · ${t("Refresh existing Topic Page")} · ${candidate.refreshTarget.title}`
      : `${candidate.namespace} · ${pages.length} ${t("Source Pages")} · ${t("Create new Topic Page")}`;
    topicExtractionReason.textContent = candidate.reason || t("No Topic rationale was supplied.");
    topicExtractionTitle.textContent = candidate.title || t("Topic Page proposal");
    topicExtractionProposal.replaceChildren(previewBlock(candidate.content || t("No content projection"), "text/markdown"));
    let activeIndex = 0;
    const select = async (index) => {
      activeIndex = index;
      topicExtractionTabs.replaceChildren(...pages.map((page, pageIndex) => (
        topicSourceTab(page, pageIndex, pageIndex === activeIndex, select)
      )));
      topicExtractionPage.replaceChildren(element("div", "loading", t("Loading full Page…")));
      await loadComparedRevision(
        pages[activeIndex],
        topicExtractionPage,
        () => activeIndex === index && topicExtractionDialog.open,
      );
    };
    if (!topicExtractionDialog.open) topicExtractionDialog.showModal();
    topicExtractionDialog.scrollTop = 0;
    return select(activeIndex);
  }

  document.querySelectorAll(".detail-tab").forEach((tab) => {
    tab.addEventListener("click", () => activate(tab.dataset.detailView));
  });
  backButton.addEventListener("click", () => back());
  document.getElementById("dialog-close").addEventListener("click", () => {
    navigationHistory.length = 0;
    dialog.close();
  });
  document.getElementById("relation-comparison-close").addEventListener("click", () => {
    relationComparisonDialog.close();
  });
  relationComparisonAccept.addEventListener("click", () => {
    if (!comparisonAcceptAction) return;
    comparisonAcceptAction();
    relationComparisonDialog.close();
  });
  relationComparisonReject.addEventListener("click", () => {
    if (!comparisonRejectAction) return;
    comparisonRejectAction();
    relationComparisonDialog.close();
  });
  relationComparisonSkip.addEventListener("click", () => {
    if (!comparisonSkipAction) return;
    comparisonSkipAction();
    relationComparisonDialog.close();
  });
  relationComparisonDialog.addEventListener("close", () => {
    comparisonAcceptAction = null;
    comparisonRejectAction = null;
    comparisonSkipAction = null;
    unlockRelationComparisonScroll();
  });
  document.getElementById("topic-extraction-review-close").addEventListener("click", () => {
    topicExtractionDialog.close();
  });

  return { open, compareRelation, reviewTopic };
}
