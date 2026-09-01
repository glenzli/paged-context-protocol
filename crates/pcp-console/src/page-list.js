// List-only presentation: never change the stored payload or tenant facets.
const text = (value) => typeof value === "string" ? value.trim() : "";
const compact = (value) => value.replace(/\s+/g, " ").trim();

export const PAGE_ROLE_LABELS = Object.freeze({
  condensed: "Condensed summary",
  covered_source: "Summarized source",
  other: "Other pages",
});

export function pageRoleBadge(hit) {
  // Only trust Runtime's structural metadata, not a tenant's kind/title/facets.
  if (hit.contentRole === "condensed") return {
    role: "condensed", label: PAGE_ROLE_LABELS.condensed,
    description: "A condensed Page used as the retrieval entry for its source Pages. Originals are retained.",
  };
  if (hit.contentRole === "covered_source") return {
    role: "covered_source", label: PAGE_ROLE_LABELS.covered_source,
    description: "This current Revision is covered by a condensed summary. Original content is retained here.",
  };
  return null;
}

export function appendPageFilters(params, {role, withSummary}) {
  if (Object.hasOwn(PAGE_ROLE_LABELS, role)) params.set("role", role);
  if (withSummary) params.set("withSummary", "true");
  return params;
}

export function pageListPreview(hit, snippet) {
  const content = text(snippet);
  const lines = content.split(/\r?\n/);
  const first = lines[0] || "";
  const markdown = /^(text\/(?:x-)?markdown)(?:;|$)/i.test(hit.previewPayload?.mediaType || "");
  const heading = markdown ? first.match(/^ {0,3}#{1,6}\s+(.+?)(?:\s+#+)?\s*$/)?.[1] : "";
  const title = compact(text(hit.facets?.title) || text(heading));
  // Only remove a whole matching first line, not a shared sentence prefix.
  const duplicate = title && compact(text(heading) || first) === title;
  return { title, excerpt: compact(duplicate ? lines.slice(1).join("\n") : content) };
}

export function pageCount(total, limit) {
  return Math.max(1, Math.ceil(Math.max(0, Number(total) || 0) / Math.max(1, limit)));
}

export function pageJump(value, total, limit) {
  if (!/^[1-9]\d*$/.test(String(value).trim())) return null;
  const page = Number(value);
  return Number.isSafeInteger(page) && page <= pageCount(total, limit) ? page : null;
}
