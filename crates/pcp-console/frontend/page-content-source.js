import DOMPurify from "dompurify";
import katex from "katex";
import "katex/dist/katex.min.css";
import { marked } from "marked";

import { protectMath } from "./math-delimiters.mjs";

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

function isMarkdown(mediaType) {
  const normalized = String(mediaType || "").split(";", 1)[0].trim().toLowerCase();
  return normalized === "text/markdown" || normalized === "text/x-markdown" || normalized.endsWith("+markdown");
}
