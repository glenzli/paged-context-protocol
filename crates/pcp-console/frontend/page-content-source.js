import DOMPurify from "dompurify";
import katex from "katex";
import "katex/dist/katex.min.css";
import { marked } from "marked";

import { protectMath } from "./math-delimiters.mjs";
import { describePagePayload, pagePayloadPreviewText } from "./page-payload.mjs";

export { describePagePayload, pagePayloadPreviewText };

marked.setOptions({
  breaks: true,
  gfm: true,
});

export function renderPageContent(target, source, mediaType = "text/markdown") {
  const content = String(source || "");
  target.replaceChildren();

  if (!isMarkdown(mediaType)) {
    const pre = document.createElement("pre");
    pre.textContent = content || "No payload projection";
    target.append(pre);
    return;
  }

  const { protectedMarkdown, math } = protectMath(content);
  target.classList.add("rich-content");
  target.innerHTML = DOMPurify.sanitize(marked.parse(protectedMarkdown), {
    USE_PROFILES: { html: true },
    ADD_ATTR: ["target"],
  });

  for (const [index, item] of math.entries()) {
    const token = target.querySelector(`[data-pcp-math="${index}"]`);
    if (!token) continue;
    try {
      katex.render(item.expression, token, {
        displayMode: item.display,
        output: "htmlAndMathml",
        strict: "ignore",
        throwOnError: false,
        trust: false,
      });
    } catch {
      token.textContent = item.display ? `$$${item.expression}$$` : `$${item.expression}$`;
    }
  }

  for (const link of target.querySelectorAll("a[href]")) {
    link.target = "_blank";
    link.rel = "noopener noreferrer";
  }
  for (const image of target.querySelectorAll("img")) {
    image.loading = "lazy";
    image.decoding = "async";
  }
}

export function renderPagePreview(target, source, mediaType = "text/markdown", options = {}) {
  const presentation = describePagePayload(source, mediaType);
  target.replaceChildren();
  target.classList.remove("rich-content", "structured-page-content");
  if (presentation.type === "markdown" || presentation.type === "raw") {
    renderPageContent(target, presentation.content, mediaType);
    return;
  }

  target.classList.add("structured-page-content");
  if (presentation.type === "packed_page") {
    renderPackedPage(target, presentation, options);
  } else {
    renderJsonFields(target, presentation.value);
  }
}

function renderPackedPage(target, packedPage, options = {}) {
  const heading = element("div", "structured-heading");
  const title = element("div", "structured-heading-title");
  title.append(
    element("strong", "", "Conversation pack"),
    element("span", "muted", `${packedPage.entries.length} entries`),
  );
  heading.append(title);
  target.append(heading);
  const entries = element("div", "packed-entries");
  for (const [index, entry] of packedPage.entries.entries()) {
    const item = element("details", "packed-entry");
    item.dataset.role = String(entry.role || "entry").toLowerCase();
    if (index === 0) item.open = true;
    const header = element("summary", "packed-entry-header");
    const identity = element("span", "packed-entry-identity");
    const timestamp = entry.createdAt || entry.pageId;
    const time = element("span", "packed-entry-time", compactTimestamp(timestamp));
    time.title = timestamp;
    identity.append(element("strong", "packed-entry-role", entry.role || "Entry"), time);
    header.append(identity, element("span", "packed-entry-preview", compactPreview(entry.content, 150)));
    const body = element("div", "structured-body");
    renderPageContent(body, entry.content, entry.mediaType);
    item.append(header, body);
    entries.append(item);
  }
  if (packedPage.entries.length) {
    const controls = options.packedEntryControls;
    if (controls && packedPage.entries.length > 1) {
      const toggle = element("button", "packed-entry-toggle-all");
      toggle.type = "button";
      const sync = () => {
        const allOpen = [...entries.querySelectorAll("details")].every((item) => item.open);
        const label = allOpen ? controls.collapseAll : controls.expandAll;
        toggle.replaceChildren(packedToggleIcon(allOpen));
        toggle.title = label;
        toggle.setAttribute("aria-label", label);
        toggle.setAttribute("aria-expanded", String(allOpen));
      };
      toggle.addEventListener("click", () => {
        const allOpen = [...entries.querySelectorAll("details")].every((item) => item.open);
        for (const item of entries.querySelectorAll("details")) item.open = !allOpen;
        sync();
      });
      entries.addEventListener("toggle", sync, true);
      heading.append(toggle);
      sync();
    }
    target.append(entries);
  } else target.append(element("div", "empty", "No readable entries"));
}

function packedToggleIcon(allOpen) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const paths = allOpen
    ? ["M3 8h5V3", "M21 8h-5V3", "M3 16h5v5", "M21 16h-5v5"]
    : ["M8 3H3v5", "M16 3h5v5", "M8 21H3v-5", "M16 21h5v-5"];
  for (const d of paths) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

function compactTimestamp(value) {
  const raw = String(value || "");
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})/.exec(raw);
  return match ? `${match[1]} ${match[2]}` : raw;
}

function compactPreview(value, limit) {
  const compact = String(value || "").replace(/\s+/g, " ").trim();
  return compact.length > limit ? `${compact.slice(0, limit)}…` : compact;
}

function renderJsonFields(target, value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    target.append(element("pre", "", JSON.stringify(value, null, 2)));
    return;
  }
  const fields = Object.entries(value);
  if (!fields.length) {
    target.append(element("div", "empty", "Empty object"));
    return;
  }
  const details = element("dl", "structured-metadata");
  for (const [label, fieldValue] of fields) {
    details.append(
      element("dt", "", label),
      element(
        "dd",
        typeof fieldValue === "object" && fieldValue !== null ? "mono" : "",
        typeof fieldValue === "object" && fieldValue !== null
          ? JSON.stringify(fieldValue, null, 2)
          : String(fieldValue ?? ""),
      ),
    );
  }
  target.append(details);
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function isMarkdown(mediaType) {
  const normalized = String(mediaType || "").split(";", 1)[0].trim().toLowerCase();
  return normalized === "text/markdown" || normalized === "text/x-markdown" || normalized.endsWith("+markdown");
}
