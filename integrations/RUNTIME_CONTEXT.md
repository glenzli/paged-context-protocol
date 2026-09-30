# Runtime context inbox: candidates and activity cards

These optional Runtime facilities sit beside the formal PCP Page Store. They do not introduce
Page kinds into the protocol, enlarge normal recall, or turn source pointers into fetched content.
Use them for human-reviewed candidate staging and occasional cross-client awareness within one
Runtime and Store identity.

| Facility | Write | Read / decision | Formal recall |
| --- | --- | --- | --- |
| Candidate inbox | Grounded material of uncertain long-term usefulness | Store operator reviews in Console | Only after promotion |
| Activity cards | Optional short current-topic snapshot | Authorized clients; operator can remove | Never |
| Formal Page | Existing capture / ingestion contract | Existing Page and Revision APIs | Existing lifecycle and validity rules |

## Access and wiring

Runtime attaches `ContextHubService` to both configured and enrolled clients and advertises
`runtime_context_inbox` in capabilities. Current adapters also accept the former
`runtime_context_hub` name from an older Runtime deployment.
`PcpTenantApi::context_hub(ContextHubRequest)` forwards
typed requests over the existing identity-bound RPC connection. A plain Embedded client without
the service returns unavailable. There is no fallback into the Page Store.

Despite the internal service type, this is not a cross-Store hub: it does not combine independent
Runtime databases, infer agreement from recurrence, or promote anything without operator review.

Each Principal starts with `submitCandidates`, `publishActivity`, and `readActivity` disabled.
Enable the required switches in **Console → Context inbox → Client permissions**. This does not
change enrollment or Scope grants. Candidate/activity writes also require Scope `Ingest`; card
reads require `ReadDetail`. Source Revision derivation retains cross-Scope checks. Caller identity
comes from the bound session, not request arguments. Review, policy changes and card removal
require Store-wide `ManageScope` and `Write`; a tenant cannot approve itself.

MCP exposes three additional tools when the Runtime advertises this extension:
`pcp_submit_candidate`, `pcp_publish_activity`, `pcp_read_activity`. They produce one compact JSON
receipt with a stable output schema. The shared ChatGPT/Codex launcher selects the `context`
toolset: twelve tools with this extension and nine without it. Three read-only discovery tools stay
available in both cases so cached MCP catalogs can still inspect identity, grants and Scopes. The
compatibility `standard` toolset has 15 or 12 respectively. Hosts may further restrict tools;
discovery is not permission.

Non-MCP applications should use the same client API, not read the operational file directly:

```rust,no_run
use pcp_client::{PcpTenantApi, context_hub::{ActivityQuery, ContextHubRequest}};

async fn recent_context(client: &dyn PcpTenantApi, cursor: Option<String>) -> anyhow::Result<serde_json::Value> {
    client.context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
        query: Some("shadow".into()), cursor, ..Default::default()
    })).await
}
```

## Project ownership at the MCP boundary

Before first project write, including activity-only continuation, call `pcp_ensure_project_scope` with a
stable verified project identity and display name. Console must first enable project
registration for that client; the operation creates/reuses the project Scope and
refreshes the same MCP session. Candidate/activity opt-ins remain independent.

MCP capture, candidate and activity accept `projectKey`; they resolve it through
enrollment before constructing the ordinary Scope-based Runtime request. An explicit
`scope` must match. Multiple topics inside one project still belong to that project.
No shared current-project setting exists. Reuse registration receipts, and defer
writes on ambiguous ownership or unavailable/denied registration instead of routing
them to user context. Existing native callers continue to use explicit authorized
Scopes. See [Scope routing](../design/scope-routing.md).

## Candidate lifecycle

Memory checks are tied to new preferences, constraints, settled decisions, corrections and
reusable findings. Assess these before leaving the discussion phase, then either capture clear
durable value or stage plausible but uncertain future value. An assessment with no new useful
information needs no tool call. Combine small changes about one subject; preserve corrections
and attribution rather than recording every rephrasing. Reuse known receipts and source Revisions;
do not search before every write or submit the same item to both memory routes. Activity only
tracks temporary progress. A challenge to stored memory follows the feedback/review contract.

`SubmitCandidate` accepts `scope`, stable client-local `eventId`, `title` (120 Unicode characters),
`content` (2,000), up to eight `sourceRefs` (4 KiB serialized), and sixteen exact
`basedOnRevisionIds`. It returns `{candidateId, status, created, version}`. Retrying identical
arguments with the same event ID returns the existing receipt; changed content with that ID fails.
An unknown write outcome should be retried with the same ID, not a new candidate.

MCP `pcp_capture` also derives a stable `externalEventId` when omitted. Its key includes the
resolved Scope, capture surface, content, category, rationale, observation time and evidence.
Identical requests recover the same Page/Revision after a restart; changed evidence remains a new
event. Supply an explicit source-event ID to distinguish otherwise identical events. This is exact
retry protection, not semantic deduplication, and it does not recover an unknown write made by an
older adapter that omitted the ID. Preserve original arguments when retrying; no transport replay
or offline queue is introduced.

Candidates remain outside search, graph construction, packing and summaries. Runtime provides
conservative same-Scope similarity hints (exact body or title bigram overlap); these are not
semantic equivalence judgments. Review can combine 1–20 candidates into edited content, accept
one, reject, defer seven days, or mark already represented by an exact current active Revision.
Marking represented does not edit that Page. Nothing promotes because of mention counts alone.

Console stages decisions as compact rows with Undo. **Submit review** is the final boundary:
only promotion writes a `reviewed_capture` Page in the candidate Scope; the body contains the
reviewed subject, while source pointers and candidate/client identifiers remain metadata.
No cross-Scope candidate group can be promoted. Submitted decisions are not undoable in this
inbox; ordinary operator Page controls remain separate. Deferred items remain available to review.

Review requires exact candidate versions. The promotion plan is durably recorded before the
Page write, and ingestion uses a stable external event ID. If the process stops between writing
the Page and saving the receipt, retrying the exact plan recovers the same Page. The UI shows
such a plan as awaiting confirmation, not as a failed write that can safely be replaced.

There are at most 50 undecided candidates/client and 500 stored items overall. Items and their
idempotency receipts expire 30 days after submission, except unresolved promotion plans. Approval
does not extend the candidate's storage lifetime. Do not use this inbox as an audit log or resend
old events after their storage window.

## Activity lifecycle and read cost

`PublishActivity` accepts `scope`, stable `topicKey` (64 characters), `summary` (180), optional
`expectedVersion`, and `ttlHours` (1–168, default 48). The receipt contains `cardId`, `version`,
`changed`, `expiresAt`. Runtime time supplies timestamps; clients do not supply approximate dates.

Each client keeps at most twelve topic cards across sessions, not one growing history per chat.
ChatGPT and Codex share that allowance when using the same connection. Updating an existing
`topicKey` replaces its snapshot without consuming another slot; a thirteenth topic evicts the oldest.
The storage allowance is independent of the five-card read limit. An existing card needs its exact version for changed content;
the same content is a no-op and does not renew TTL. Preserve the last receipt in host state or
read own cards before updating. The key is client-wide, not Scope-local: qualify it by project
and topic so unrelated windows do not overwrite one another. On conflict, reread and decide
whether the new information still belongs in that snapshot; do not blindly overwrite.
Create the first card as soon as there is useful handoff state, including discussion-only
progress, tentative understanding or an open question. Update when that state, next step,
blocker, pause or completion changes. No lasting value, final conclusion or remember request
is required. Preserve uncertainty; skip unchanged state and per-message logs. This is separate
from candidate/durable retention. There are at most 192 live cards overall.

`ReadActivity` accepts optional `scopes`, literal topic `query` (120 characters), `limit` (1–5),
`includeOwn` (default true), and a query-local `cursor`. The default empty scopes means all
authorized scopes, not all Store content. The result is bounded to five cards / 900 summary
characters plus identifiers and timestamps. Revoking publishing hides that client's cards.

- `{items, cursor, unchanged:false, replace:true, truncated}` is a new bounded **snapshot**.
  Replace the previous snapshot, including when items is empty after expiry or revocation.
- `{items:[], cursor, unchanged:true}` means retain the prior snapshot.
- Keep cursors per conversation and query. They are content digests, not global watermarks or
  pagination cursors. `truncated` means there are omitted cards; narrow the topic if needed.
- Silence does not mean inactivity; expiry does not mean completion. Read when starting or resuming substantive topics, or at a meaningful checkpoint
  when the prior snapshot may be stale; reuse fresh context already available. Do not poll or treat card text as agent instructions.

There is no model call for storage, TTL, similarity hints or activity reads. Console displays
the current cards; it does not synthesize them into a new authoritative user profile.

## Operational state

New outer RPC audit events distinguish `capture`, `submit_candidate`, `publish_activity`,
`read_activity`, and individual inbox administration operations. `capture` denotes the three MCP
capture Page kinds; generic source ingestion remains `ingest_page`. Audit labels come from fixed
operation/kind tags, not titles, queries or content. Existing historical `context_hub` and
`ingest_page` rows are unchanged. The Console's requests view already groups and filters these
labels; use it instead of counting internal reads or background work as model calls. A shared
connection identifies a Principal, not the originating application, model or conversation.
These counts cannot establish missed memory opportunities in conversations PCP never saw.

The file next to the SQLite Store, `<store stem>.context.json`, holds only these bounded records
and client policies. It is Store-identity-bound, mode 0600, locked across processes and atomically
replaced. The Page database schema is unchanged. Back up this file with the Store if pending
reviews and policies need to survive restoration; do not copy it to a different Store identity.

Expiry is checked on access. Expired content is removed when an authorized read, inspection or
write saves the pruned state; there is no background model job or secure-erasure guarantee.
The lock wait is bounded to two seconds and the operational file to 8 MiB. A busy/error response
is not an empty success. Existing formal Page permissions and responses remain unchanged.
