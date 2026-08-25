const PACKED_PAGE = "application/vnd.pcp.packed-page+json";

export function normalizeMediaType(mediaType) {
  return String(mediaType || "").split(";", 1)[0].trim().toLowerCase();
}

export function describePagePayload(source, mediaType) {
  const content = String(source || "");
  const normalized = normalizeMediaType(mediaType);
  if (isMarkdown(normalized)) return { type: "markdown", content };

  const value = parseJson(content, normalized);
  if (value === null) return { type: "raw", content };
  if (normalized === PACKED_PAGE && isRecord(value)) {
    return {
      type: "packed_page",
      entries: Array.isArray(value.entries) ? value.entries.map(packedEntry).filter(Boolean) : [],
    };
  }
  return { type: "json", value };
}

export function pagePayloadPreviewText(source, mediaType) {
  const presentation = describePagePayload(source, mediaType);
  switch (presentation.type) {
    case "packed_page":
      return presentation.entries
        .slice(0, 3)
        .map((entry) => joinPreview([roleLabel(entry.role), entry.content]))
        .filter(Boolean)
        .join(" · ");
    case "json":
      return firstUsefulText(presentation.value) || JSON.stringify(presentation.value);
    default:
      return presentation.content;
  }
}

function packedEntry(value) {
  if (!isRecord(value) || !isRecord(value.payload)) return null;
  return {
    pageId: text(value.pageId),
    role: isRecord(value.facets) ? text(value.facets.role) : "",
    createdAt: text(value.createdAt || value.observedAt),
    mediaType: text(value.payload.mediaType) || "text/plain",
    content: text(value.payload.content),
  };
}

function parseJson(content, mediaType) {
  if (!(mediaType === "application/json" || mediaType.endsWith("+json"))) return null;
  try {
    return JSON.parse(content);
  } catch (_) {
    return null;
  }
}

function firstUsefulText(value) {
  if (!isRecord(value)) return typeof value === "string" ? value : "";
  for (const key of ["title", "summary", "content", "description", "caption", "filename"]) {
    const valueText = text(value[key]);
    if (valueText) return valueText;
  }
  return "";
}

function roleLabel(role) {
  if (!role) return "";
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function joinPreview(values) {
  return values.filter(Boolean).join(" — ");
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isMarkdown(mediaType) {
  return mediaType === "text/markdown" || mediaType === "text/x-markdown" || mediaType.endsWith("+markdown");
}
