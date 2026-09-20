// Display names only. Raw identifiers remain unchanged in requests and diagnostics.
const OPERATIONS = {
  read_pages: "Read memories", get_page: "Open memory", list_pages: "Browse memories",
  browse_content_pages: "Browse memories", browse_index: "Browse memory index", ingest_page: "Import content",
  context_hub: "Context inbox operations", inspect_context_inbox: "Inspect context inbox",
  integrity_check: "Check storage integrity", list_scopes: "List memory scopes",
  query_access_log: "Read access records", page_count: "Count memories",
  content_library_summary: "Inspect memory statistics", query_audit_summary: "Inspect query statistics",
  search_pages: "Search memories", semantic_search: "Semantic search", match_intent: "Intent matching",
  context_query: "Build context pack", describe: "Inspect service", health_check: "Health check",
  write_page: "Write memory", create_page: "Create memory", update_page: "Update memory",
  capture: "Capture evidence", submit_candidate: "Submit candidate", submit_candidates: "Submit candidates",
  organize_candidates: "Organize candidates", review_candidate_synthesis: "Review memory drafts",
  review_candidate_synthesis_escalated: "Review memory drafts in depth", extract_topic: "Extract topics",
  verify_maintenance: "Verify maintenance proposals", verify_maintenance_escalated: "Verify maintenance proposals in depth",
  assess_archive: "Review archive proposals", review_update: "Review memory updates",
  verify_relation: "Verify memory relations", select_relation: "Find related memories",
  summarize: "Summarize content", pack: "Pack source content", reconcile: "Reconcile feedback",
  review_summary: "Review summaries", review_topic: "Review topics",
  publish_activity: "Publish activity", read_activity: "Read activity", archive_page: "Archive memory",
};
const KINDS = {
  reviewed_capture: "Reviewed memory", chatgpt_capture: "Conversation memory",
  topic_summary: "Topic summary", summary_projection: "Attached summary",
  document: "Document", conversation: "Conversation", source_bundle: "Source pack",
};

export const operationLabel = (value, t = value => value) => OPERATIONS[value] ? t(OPERATIONS[value]) : value;
export const kindLabel = (value, t = value => value) => KINDS[value] ? t(KINDS[value]) : value;

export function technicalDetails(element, label, rows) {
  const details = element("details", "technical-details");
  details.append(element("summary", "", label));
  const values = element("dl", "details-grid technical-fields");
  for (const [name, value] of rows) {
    if (value === undefined || value === null || value === "") continue;
    values.append(element("dt", "", name), element("dd", "mono", String(value)));
  }
  details.append(values);
  return details;
}

// Update only label text when scope metadata arrives; keep editors and open details intact.
export function scopeLabel(element, namespaces, resolve, tag = "span", className = "") {
  const node = element(tag, className, namespaces.map(resolve).join(", "));
  node.setAttribute("data-scope-names", JSON.stringify(namespaces));
  return node;
}

export function refreshScopeLabels(root, resolve) {
  for (const node of root.querySelectorAll("[data-scope-names]")) {
    node.textContent = JSON.parse(node.getAttribute("data-scope-names")).map(resolve).join(", ");
  }
}
