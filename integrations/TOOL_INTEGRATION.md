# Model-facing PCP tools

PCP's program API and a model's tool result serve different consumers. Keep the authorized API
result for application logic or diagnostics; give the model a bounded evidence projection.
This guide describes the shared Rust interface and the MCP 0.2 tool presentation. It does not
change the v0.8 Store/RPC response contract or grant new permissions.

## Recommended call path

```text
Model tool arguments
  -> host validation and bound limits
  -> enrolled RemotePcpClient / EmbeddedPcpClient (PcpTenantApi)
  -> authorized typed result
  -> pcp_client::model_context
  -> compact JSON or text for the model
```

Use a dedicated enrolled Principal. Normal clients use `contribute` for selective ingestion and
feedback, plus explicitly granted read access; do not embed operator credentials in model tools.
Projection is not an access-control boundary: only pass results from the authorized client.
Keep remote sources tenant-owned; returning a SourceRef does not fetch or parse its contents.

## Reusable response interface

`pcp-client::model_context` owns deterministic presentation, with no model calls, Store access or
state. It is usable by applications such as Symbiont without depending on MCP:

| API result | Projection function |
| --- | --- |
| `SearchResult` (search or browse) | `search_context` |
| `Vec<ReadPage>` | `read_context` |
| `QueryContextResponse` (semantic search or intent matching) | `query_context` |
| `GraphSliceResponse` | `graph_context` |

All functions return `ModelContext`: `items`, an optional `nextCursor`, a `truncated` flag,
optional graph `edges` and optional `stoppedReason`. Use `serde_json::to_string` for JSON or
`ModelContext::to_text()` for text. Both retain evidence identifiers and caveats; text is not a
second model-generated summary. Render **one** representation into the model context.

```rust,no_run
use pcp_client::{PcpTenantApi, model_context::{self, ContextBudget, ContextView}};
use pcp_core::ReadPagesRequest;

async fn read_for_model(
    client: &dyn PcpTenantApi,
    revision_ids: Vec<String>,
) -> anyhow::Result<String> {
    let view = ContextView::Content;
    let raw = client.read_pages(ReadPagesRequest {
        page_ids: vec![], revision_ids,
        projections: view.projections(), // includes Validity, even for content
        max_chars: 8_000,
    }).await?;
    let evidence = model_context::read_context(&raw, view, ContextBudget::default());
    Ok(evidence.to_text())
}
```

`ContextBudget` defaults to 8,000 total content characters and 400 characters per search/query
preview. Counts are Unicode scalar values, not bytes or tokens. Hosts may choose another budget
or `usize::MAX` to avoid further clipping already-bounded content. Budgets do not shorten IDs,
dates, validity caveats or cursors. They are **not** a total serialized-response limit; bound the
upstream item count, graph size and explicit history requests as well. Do not silently discard
items and forward a cursor that skips them.

## Default evidence and follow-up

An item carries stable `pageId`, exact `revisionId`, `scope`, `kind`, content when available,
`detail` (`payload`, `summary`, `excerpt`, `reference`) and `truncated`. Applicable observation/
effective dates, lifecycle state and validity caveats remain present. Read results identify a
different current Page head without silently replacing the requested old snapshot.

- Search/query previews are candidate evidence. Read useful exact Revisions before relying on
  their body. Summary-derived hits retain their Summary Revision when supplied by the API.
- `ContextView::Content` returns the body and caveats;
  `Context` adds relations; `Sources` adds source coordinates and basis Revision IDs;
  `History` adds Revision/assessment identifiers; `Full` combines these. These are evidence views,
  not dumps of every internal field. Use the original program API for full audit objects/facets.
- No validity assessment means unassessed, not confirmed correct. Preserve qualified/disputed/
  superseded/retracted state and its scope/reason; retrieve the assessment Revision for detail.
- Source metadata or historical content not supplied by the API is never invented by the adapter.
- `truncated=true` means some text or graph coverage is incomplete. Read a selected exact Revision
  with a sufficient budget rather than assuming a short excerpt is complete. A cursor means there
  is another page of candidates, not that its content has already been examined.
- Timeouts and permission failures stay errors. Never turn them into an empty successful list.
  `stoppedReason` preserves Router stop information without exposing its full accounting trace.

## MCP surface

MCP discovery is deliberately smaller than the complete API. `PCP_MCP_TOOLSET` selects one fixed
catalog for a server process:

| Toolset | Tools | Intended use |
| --- | ---: | --- |
| `core` (default) | 5 | literal/semantic search, exact read, durable capture and feedback |
| `context` | 8–11 | `core`, read-only discovery, plus candidate/activity tools when Runtime supports the inbox |
| `standard` | 11–14 | compatibility surface with diagnostics, Scope listing and advanced retrieval |
| `maintenance` | all available | trusted operator and development workflows |

The bundled Codex and ChatGPT launchers select `context`; without the Runtime inbox extension it
contracts to the same five tools as `core`. `standard` preserves the former ordinary catalog for
integrations that explicitly need it. Backend permissions still apply to every call, and ordinary
capture/feedback approval policy belongs to the host (the Codex plugin prompts for both). Do not
treat discovery or a hidden tool as an authorization boundary.

When exposed by `context`, `standard` or `maintenance`, `pcp_describe.capabilities` is the provider-backend
inventory: Store operations plus optional Runtime extensions. Feature names such as
`access_audit` and `revision_retention_planning` are not MCP tool names and do not mean that a
standard client can call maintenance operations. `pcp_describe.mcpSurface.availableTools` is the
exact catalog for that server instance; MCP `tools/list` remains authoritative for discovery.

The `context` surface also keeps `pcp_whoami` and `pcp_list_scopes` callable so an agent can resolve
a real grant or namespace ambiguity. They are not a required preamble for ordinary recall: search
without explicit Scopes uses the server-injected access session, and Runtime permissions remain
authoritative.

When Runtime advertises `runtime_context_inbox`, the `context`, `standard`, and `maintenance`
toolsets can expose three optional candidate/activity tools. They use a separate, bounded
operational store and per-client opt-in. See [Runtime context](RUNTIME_CONTEXT.md) for API,
permissions, retries and snapshot reads. They must not fall back to formal capture when disabled,
and activity writes are never mandatory. This Runtime-local inbox is unrelated to Revision payload
retention plans and leases.

Recommended model flow:

1. `pcp_semantic_search` for prior decisions/preferences/constraints; `pcp_search_pages` for literal
   text or known IDs. Do not query when supplied evidence already settles a self-contained task.
2. `pcp_read_pages` on selected `revisionIds`. Use `view` to request only the follow-up needed.
3. On a `standard` surface, use `pcp_expand_graph` only from relevant anchors and
   `pcp_match_intent` for a material unresolved retrieval question, initially at low effort. Do not
   require these specialist tools for ordinary recall; stop searching when results add no value.
4. `pcp_capture` only for confirmed reusable context or requested retention. Use
   `pcp_submit_feedback` for explicit corrections; feedback remains pending Console review.

Retrieval tools accept `format=json` (default) or `format=text`. MCP returns **one text content
block**, containing compact JSON or evidence text. It does not duplicate retrieval bodies in
`structuredContent` or advertise the full Store output schema on every tool. Small write receipts
still expose structured results with stable IDs. New feedback says `status=pending_review`;
an idempotent replay says `existing_feedback` and does not claim its current review state.

**Migration from MCP 0.1:** retrieval results now have `items` instead of raw `hits`, `pages` or
`entries`. Read the content block; do not require `structuredContent` for retrieval. Parameter names
remain compatible, with optional `format` added and `sources`/`history` views added. If a host
requires raw API objects, use the typed client API rather than parsing model-facing MCP output.
MCP server initialize reports presentation version `0.2.0`; this is not the PCP protocol version.

Keep universal server instructions short: some hosts repeat them before every tool. Put parameter
semantics beside the relevant tool and detailed write examples in an on-demand Skill/reference.
For hosts without Skills, retain essential capture/feedback rules in those tools' own descriptions.
Do not copy both a full raw result and its compact representation into the prompt. Raw traces, if
retained at all, need the same access protections as the underlying content and must stay outside
model context by default.

## Compatibility and validation

This layer adds no RPC method and changes no Store data. Existing applications keep their current
responses until they opt into projection. Non-Rust hosts may implement the same field policy above
after their authorized API call; do not rely on JSON key deletion without checking semantics.

Run the client projection tests and MCP wire tests when adapting tools. They cover Unicode budgets,
exact Revision/date/caveat preservation, next cursors, source/history views, unchanged raw objects,
single-channel MCP bodies, inaccessible evidence and undisclosed management calls. Keep a tool
catalog size budget and representative response-size checks alongside behavior tests.
