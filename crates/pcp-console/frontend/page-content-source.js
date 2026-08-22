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
  if (presentation.type === "external_signal") {
    renderExternalSignal(target, presentation);
  } else if (presentation.type === "packed_page") {
    renderPackedPage(target, presentation, options);
  } else if (presentation.type === "image_asset") {
    renderImageAsset(target, presentation, options);
  } else {
    renderJsonFields(target, presentation.value);
  }
}

function renderExternalSignal(target, signal) {
  if (signal.title) target.append(element("h4", "structured-title", signal.title));
  if (signal.summary) {
    target.append(
      element("div", "structured-label", "Summary"),
      element("p", "structured-lead", signal.summary),
    );
  }
  if (signal.content) {
    const body = element("div", "structured-body");
    renderPageContent(body, signal.content, "text/markdown");
    target.append(element("div", "structured-label", "Observation"), body);
  }
  appendMetadata(target, [
    ["Event", signal.eventAt],
    ["Qualification", signal.qualificationNote],
    ["Review", signal.reviewReason],
  ]);
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
    if (index === 0) item.open = true;
    const header = element("summary", "packed-entry-header");
    header.append(
      element("strong", "", entry.role || "Entry"),
      element("span", "muted", entry.createdAt || entry.pageId),
      element("span", "packed-entry-preview", compactPreview(entry.content, 150)),
    );
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
        toggle.textContent = allOpen ? controls.collapseAll : controls.expandAll;
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

function compactPreview(value, limit) {
  const compact = String(value || "").replace(/\s+/g, " ").trim();
  return compact.length > limit ? `${compact.slice(0, limit)}…` : compact;
}

function renderImageAsset(target, image, options) {
  if (options.mediaUrl) {
    const preview = document.createElement("img");
    preview.className = "local-media-preview";
    preview.src = options.mediaUrl;
    preview.alt = image.filename || "Referenced image";
    preview.loading = "lazy";
    preview.decoding = "async";
    preview.addEventListener("error", () => {
      preview.replaceWith(element("div", "media-unavailable muted", "Local preview unavailable"));
    }, { once: true });
    target.append(preview);
  }
  target.append(element("h4", "structured-title", image.filename || "Image asset"));
  appendMetadata(target, [
    ["Media type", image.mimeType],
    ["Dimensions", image.width && image.height ? `${image.width} × ${image.height}` : ""],
    ["Size", image.byteSize === null ? "" : formatBytes(image.byteSize)],
  ]);
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

function appendMetadata(target, rows) {
  const populated = rows.filter(([, value]) => value !== "" && value !== null && value !== undefined);
  if (!populated.length) return;
  const details = element("dl", "structured-metadata");
  for (const [label, value] of populated) {
    details.append(element("dt", "", label), element("dd", "", value));
  }
  target.append(details);
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function isMarkdown(mediaType) {
  const normalized = String(mediaType || "").split(";", 1)[0].trim().toLowerCase();
  return normalized === "text/markdown" || normalized === "text/x-markdown" || normalized.endsWith("+markdown");
}
