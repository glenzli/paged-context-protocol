const EXTERNAL_SIGNAL = "application/vnd.symbiont.external-signal+json";
const PACKED_PAGE = "application/vnd.pcp.packed-page+json";
const IMAGE_ASSET = "application/vnd.symbiont.image+json";

export function normalizeMediaType(mediaType) {
  return String(mediaType || "").split(";", 1)[0].trim().toLowerCase();
}

export function describePagePayload(source, mediaType) {
  const content = String(source || "");
  const normalized = normalizeMediaType(mediaType);
  if (isMarkdown(normalized)) return { type: "markdown", content };

  const value = parseJson(content, normalized);
  if (value === null) return { type: "raw", content };
  if (normalized === EXTERNAL_SIGNAL && isRecord(value)) {
    return {
      type: "external_signal",
      title: text(value.title),
      summary: text(value.summary),
      content: text(value.content),
      eventAt: text(value.event_at),
      qualificationNote: text(value.qualification_note),
      reviewReason: text(value.review_reason),
    };
  }
  if (normalized === PACKED_PAGE && isRecord(value)) {
    return {
      type: "packed_page",
      entries: Array.isArray(value.entries) ? value.entries.map(packedEntry).filter(Boolean) : [],
    };
  }
  if (normalized === IMAGE_ASSET && isRecord(value)) {
    return {
      type: "image_asset",
      filename: text(value.filename),
      mimeType: text(value.mimeType),
      width: finiteNumber(value.width),
      height: finiteNumber(value.height),
      byteSize: finiteNumber(value.byteSize),
    };
  }
  return { type: "json", value };
}

export function pagePayloadPreviewText(source, mediaType) {
  const presentation = describePagePayload(source, mediaType);
  switch (presentation.type) {
    case "external_signal":
      return joinPreview([presentation.title, presentation.content || presentation.summary]);
    case "packed_page":
      return presentation.entries
        .slice(0, 3)
        .map((entry) => joinPreview([roleLabel(entry.role), entry.content]))
        .filter(Boolean)
        .join(" · ");
    case "image_asset": {
      const dimensions = presentation.width && presentation.height
        ? `${presentation.width} × ${presentation.height}`
        : "";
      return joinPreview([presentation.filename, dimensions, presentation.mimeType]);
    }
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

function finiteNumber(value) {
  return Number.isFinite(value) ? value : null;
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isMarkdown(mediaType) {
  return mediaType === "text/markdown" || mediaType === "text/x-markdown" || mediaType.endsWith("+markdown");
}
