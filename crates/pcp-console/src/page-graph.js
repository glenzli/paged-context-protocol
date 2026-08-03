const SVG_NS = "http://www.w3.org/2000/svg";

export function relationFamily(relationType) {
  if (["aggregates", "derived_from", "summarizes"].includes(relationType)) return "derivation";
  if (["follows", "responds_to", "continues"].includes(relationType)) return "conversation";
  if (["supports", "contradicts", "supersedes", "qualifies", "reaffirms", "outdated_by"].includes(relationType)) return "evidence";
  return "semantic";
}

function svgElement(tag, attributes = {}) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, String(value));
  return node;
}

function layoutNodes(nodes) {
  const positions = new Map();
  const rings = new Map();
  for (const node of nodes) {
    const ring = rings.get(node.depth) || [];
    ring.push(node);
    rings.set(node.depth, ring);
  }
  positions.set(nodes.find((node) => node.depth === 0)?.pageId, { x: 0, y: 0 });

  let extent = 90;
  for (const [depth, ring] of [...rings.entries()].filter(([depth]) => depth > 0)) {
    const radius = Math.max(depth * 118, ring.length * 7.5);
    extent = Math.max(extent, radius + 30);
    ring.forEach((node, index) => {
      const angle = -Math.PI / 2 + (index * Math.PI * 2) / ring.length + depth * 0.24;
      positions.set(node.pageId, {
        x: Math.cos(angle) * radius,
        y: Math.sin(angle) * radius,
      });
    });
  }
  return { positions, extent };
}

function zoomViewBox(svg, viewBox, factor) {
  const nextWidth = viewBox.width * factor;
  const nextHeight = viewBox.height * factor;
  viewBox.x += (viewBox.width - nextWidth) / 2;
  viewBox.y += (viewBox.height - nextHeight) / 2;
  viewBox.width = nextWidth;
  viewBox.height = nextHeight;
  svg.setAttribute("viewBox", `${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`);
}

export function createTopologyMap({ topology, relationFilter, onNavigate }) {
  const section = document.createElement("section");
  section.className = "topology-section";
  const header = document.createElement("header");
  header.className = "topology-header";
  const status = document.createElement("div");
  const count = topology.truncated ? `${topology.nodes.length}+` : String(topology.nodes.length);
  status.append(
    Object.assign(document.createElement("strong"), { textContent: "Local topology" }),
    Object.assign(document.createElement("span"), {
      className: "muted",
      textContent: `${topology.directNeighborCount} direct · ${count} Pages loaded · ${topology.edges.length} relations · ${topology.depth} hops${topology.truncated ? " · partial" : ""}`,
    }),
  );
  const zoomControls = document.createElement("div");
  zoomControls.className = "topology-zoom";
  header.append(status, zoomControls);

  const visibleEdges = relationFilter === "all"
    ? topology.edges
    : topology.edges.filter((edge) => relationFamily(edge.relationType) === relationFilter);
  const visibleIds = new Set(visibleEdges.flatMap((edge) => [edge.fromPageId, edge.toPageId]));
  const root = topology.nodes.find((node) => node.depth === 0);
  if (root) visibleIds.add(root.pageId);
  const visibleNodes = topology.nodes.filter((node) => visibleIds.has(node.pageId));
  const { positions, extent } = layoutNodes(visibleNodes);
  const svg = svgElement("svg", {
    class: "topology-map",
    role: "img",
    "aria-label": `Page graph with ${visibleNodes.length} visible Pages and ${visibleEdges.length} visible relations`,
  });
  const initialViewBox = { x: -extent, y: -extent, width: extent * 2, height: extent * 2 };
  const viewBox = { ...initialViewBox };
  const applyViewBox = () => svg.setAttribute("viewBox", `${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`);
  applyViewBox();

  const edgesGroup = svgElement("g", { class: "topology-edges" });
  for (const edge of visibleEdges) {
    const from = positions.get(edge.fromPageId);
    const to = positions.get(edge.toPageId);
    if (!from || !to) continue;
    const line = svgElement("line", {
      x1: from.x,
      y1: from.y,
      x2: to.x,
      y2: to.y,
      class: `topology-edge relation-${relationFamily(edge.relationType)}`,
    });
    line.append(svgElement("title"));
    line.firstChild.textContent = edge.relationType;
    edgesGroup.append(line);
  }
  svg.append(edgesGroup);

  const nodesGroup = svgElement("g", { class: "topology-nodes" });
  const orderedNodes = [...visibleNodes].sort((left, right) => Number(left.depth === 0) - Number(right.depth === 0));
  for (const node of orderedNodes) {
    const point = positions.get(node.pageId);
    if (!point) continue;
    const current = node.depth === 0;
    const group = svgElement("g", {
      class: `topology-node${current ? " current" : ""}`,
      transform: `translate(${point.x} ${point.y})`,
      role: "button",
      tabindex: "0",
      "aria-label": current ? `Current Page ${node.pageId}` : `Open Page ${node.pageId}`,
    });
    if (current) group.append(svgElement("circle", { class: "topology-current-halo", r: 20 }));
    group.append(svgElement("circle", { class: "topology-node-dot", r: current ? 10 : 4.5 }));
    const label = current ? svgElement("text", { x: 14, y: 4 }) : null;
    if (label) {
      label.textContent = "Current";
      group.append(label);
    }
    const title = svgElement("title");
    title.textContent = `${current ? "Current Page" : `Depth ${node.depth}`} · ${node.pageId}`;
    group.append(title);
    group.addEventListener("click", () => {
      if (!current) onNavigate(node.pageId);
    });
    group.addEventListener("keydown", (event) => {
      if (!current && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault();
        onNavigate(node.pageId);
      }
    });
    nodesGroup.append(group);
  }
  svg.append(nodesGroup);

  for (const [label, title, factor] of [["−", "Zoom out", 1.25], ["Fit", "Fit graph", 0], ["+", "Zoom in", 0.8]]) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "topology-zoom-button";
    button.textContent = label;
    button.title = title;
    button.setAttribute("aria-label", title);
    button.addEventListener("click", () => {
      if (factor === 0) Object.assign(viewBox, initialViewBox);
      else zoomViewBox(svg, viewBox, factor);
      applyViewBox();
    });
    zoomControls.append(button);
  }

  svg.addEventListener("wheel", (event) => {
    event.preventDefault();
    zoomViewBox(svg, viewBox, event.deltaY > 0 ? 1.12 : 0.88);
  }, { passive: false });
  let drag = null;
  svg.addEventListener("pointerdown", (event) => {
    if (event.target.closest?.(".topology-node")) return;
    drag = { x: event.clientX, y: event.clientY, viewX: viewBox.x, viewY: viewBox.y };
    svg.setPointerCapture(event.pointerId);
    svg.classList.add("dragging");
  });
  svg.addEventListener("pointermove", (event) => {
    if (!drag) return;
    const rect = svg.getBoundingClientRect();
    viewBox.x = drag.viewX - (event.clientX - drag.x) * viewBox.width / rect.width;
    viewBox.y = drag.viewY - (event.clientY - drag.y) * viewBox.height / rect.height;
    applyViewBox();
  });
  const endDrag = (event) => {
    if (!drag) return;
    drag = null;
    svg.classList.remove("dragging");
    if (svg.hasPointerCapture(event.pointerId)) svg.releasePointerCapture(event.pointerId);
  };
  svg.addEventListener("pointerup", endDrag);
  svg.addEventListener("pointercancel", endDrag);

  section.append(header, svg);
  return section;
}
