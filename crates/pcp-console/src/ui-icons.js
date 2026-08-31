const ICON_PATHS = Object.freeze({
  access: ["M18 21a8 8 0 0 0-16 0", "M14 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0", "M22 21a8 8 0 0 0-6-7.75", "M16 3.13a4 4 0 0 1 0 7.75"],
  restart: ["M4 3v5h5", "M4.2 7.5A9 9 0 1 0 7.5 4.2", "M12 8.5v3.3", "M14.1 10.4a3 3 0 1 1-4.2 0"],
  refresh: ["M20 4v6h-6", "M19.2 9.2a8 8 0 1 0-1.5 7.5"],
  settings: ["M4 6h10", "M18 6h2", "M4 12h3", "M11 12h9", "M4 18h8", "M16 18h4", "M14 3v6", "M7 9v6", "M12 15v6"],
  open: ["M5 19 19 5", "M8 5h11v11"],
  compare: ["M4 5h6v14H4z", "M14 5h6v14h-6z", "M10 12h4", "m2-2 2 2-2 2"],
  suppress: ["m9 15-1.4 1.4a3 3 0 0 1-4.2-4.2l3.5-3.5a3 3 0 0 1 4.2 0L12.5 10", "m15 9 1.4-1.4a3 3 0 0 1 4.2 4.2l-3.5 3.5a3 3 0 0 1-4.2 0L11.5 14", "M4 4 20 20"],
  accept: ["M5 12.5 9.5 17 19 7"],
  reject: ["m8.5 15.5-1.6 1.6a3.5 3.5 0 0 1-5-5l3.2-3.2a3.5 3.5 0 0 1 4.5-.4", "m15.5 8.5 1.6-1.6a3.5 3.5 0 0 1 5 5l-3.2 3.2a3.5 3.5 0 0 1-4.5.4", "M8.8 12h6.4"],
  defer: ["M12 7v5l3 2", "M12 21a9 9 0 1 0-9-9"],
  undo: ["M9 7 4 12l5 5", "M5 12h8a6 6 0 0 1 6 6"],
  run: ["m8 5 11 7-11 7z"],
  search: ["M20 20l-4.2-4.2", "M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0"],
  scan: ["M20 20l-4.2-4.2", "M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0"],
  rescan: ["M15.5 15.5 20 20", "M17 9a8 8 0 1 1-16 0 8 8 0 0 1 16 0", "M12.5 5.5V9H9", "M12.2 8.8a3.5 3.5 0 1 1-1-2.4"],
  analyze: ["M5 3h9l5 5v13H5z", "M14 3v5h5", "M8.5 17v-3", "M12 17v-6", "M15.5 17v-4.5"],
  retry: ["M20 7v5h-5", "M18 17a8 8 0 1 1 1.2-11"],
  apply: ["M5 12.5 9.5 17 19 7"],
  skip: ["m8 5 7 7-7 7", "M16 5v14"],
  end: ["M6 6l12 12", "M18 6 6 18"],
  manual: ["M4 5h16v14H4z", "M8 9h8", "M8 13h5", "M8 17h3"],
  pack: ["M4 8h16v11H4z", "M4 12h16", "M8 5h8", "m8 16-2-2 2-2", "m16 16 2-2-2-2"],
  summary: ["M5 6h14", "M5 10h10", "M5 14h14", "M5 18h8"],
  relation: ["m9 15-1.5 1.5a3.5 3.5 0 0 1-5-5L5 9a3.5 3.5 0 0 1 5-5l1.5 1.5", "m15 9 1.5-1.5a3.5 3.5 0 0 1 5 5L19 15a3.5 3.5 0 0 1-5 5l-1.5-1.5", "M8.5 12h7"],
  reconciliation: ["M5 4h14v12H9l-4 4z", "M8 8h8", "M8 12h5", "m14.5 19 2 2 4-5"],
  topic: ["M6 3h8l4 4v14H6z", "M14 3v5h5", "M10 13h4", "M12 11v4"],
  archive: ["M4 7h16v13H4z", "M3 4h18v4H3z", "M9 12h6"],
  retain: ["M5 12.5 9.5 17 19 7"],
});

const ICON_TOOLTIP_SELECTOR = [
  ".topbar-icon-button[aria-label]",
  ".icon-button[aria-label]",
  ".compact-icon-button[aria-label]",
].join(",");

let iconTooltipObserver = null;

export function iconNames() {
  return Object.keys(ICON_PATHS);
}

export function iconDefinition(name) {
  return ICON_PATHS[name] || null;
}

export function pathIcon(paths, className = "") {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  if (className) svg.setAttribute("class", className);
  for (const d of paths) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

export function icon(name, className = "") {
  const paths = iconDefinition(name);
  if (!paths) throw new Error(`Unknown UI icon: ${name}`);
  const svg = pathIcon(paths, className);
  svg.dataset.iconName = name;
  return svg;
}

export function hydrateIcons(root = document) {
  root.querySelectorAll("[data-icon]").forEach((node) => {
    const name = node.dataset.icon;
    if (!name || node.querySelector(`svg[data-icon-name=\"${name}\"]`)) return;
    node.prepend(icon(name));
  });
}

function hydrateIconTooltipNode(node) {
  if (!(node instanceof Element)) return;
  const targets = node.matches(ICON_TOOLTIP_SELECTOR)
    ? [node]
    : [...node.querySelectorAll(ICON_TOOLTIP_SELECTOR)];
  for (const target of targets) {
    target.dataset.fastTooltip = "";
    target.removeAttribute("title");
  }
}

export function hydrateIconTooltips(root = document) {
  hydrateIconTooltipNode(root.documentElement || root);
  if (root !== document || iconTooltipObserver || typeof MutationObserver === "undefined") return;
  iconTooltipObserver = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "attributes") {
        hydrateIconTooltipNode(record.target);
        continue;
      }
      record.addedNodes.forEach(hydrateIconTooltipNode);
    }
  });
  iconTooltipObserver.observe(document.documentElement, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["aria-label", "title"],
  });
}
