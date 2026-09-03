# Runtime candidate inbox and activity cards

These optional Runtime facilities sit beside the formal PCP Page Store. They do not introduce
Page kinds into the protocol, enlarge normal recall, or turn source pointers into fetched content.
Use them for human-reviewed candidate retention and occasional cross-client awareness.

| Facility | Write | Read / decision | Formal recall |
| --- | --- | --- | --- |
| Candidate inbox | Grounded material of uncertain long-term usefulness | Store operator reviews in Console | Only after promotion |
| Activity cards | Optional short current-topic snapshot | Authorized clients; operator can remove | Never |
| Formal Page | Existing capture / ingestion contract | Existing Page and Revision APIs | Existing lifecycle and validity rules |

## Access and wiring

Runtime attaches `ContextHubService` to both configured and enrolled clients and advertises
`runtime_context_hub` in capabilities. `PcpTenantApi::context_hub(ContextHubRequest)` forwards
typed requests over the existing identity-bound RPC connection. A plain Embedded client without
the service returns unavailable. There is no fallback into the Page Store.

Each Principal starts with `submitCandidates`, `publishActivity`, and `readActivity` disabled.
Enable the required switches in **Console → Context inbox → Client permissions**. This does not
change enrollment or Scope grants. Candidate/activity writes also require Scope `Ingest`; card
reads require `ReadDetail`. Source Revision derivation retains cross-Scope checks. Caller identity
comes from the bound session, not request arguments. Review, policy changes and card removal
require Store-wide `ManageScope` and `Write`; a tenant cannot approve itself.

MCP exposes three additional tools when the Runtime advertises this extension:
`pcp_submit_candidate`, `pcp_publish_activity`, `pcp_read_activity`. They produce one compact JSON
text block, not duplicate raw and structured results. The ordinary catalog is 14 tools with this
extension, 11 without it. Hosts may further restrict tools; discovery is not permission.

Non-MCP applications should use the same client API, not read the operational file directly:

```rust,no_run
use pcp_client::{PcpTenantApi, context_hub::{ActivityQuery, ContextHubRequest}};

async fn recent_context(client: &dyn PcpTenantApi, cursor: Option<String>) -> anyhow::Result<serde_json::Value> {
    client.context_hub(ContextHubRequest::ReadActivity(ActivityQuery {
        query: Some("shadow".into()), cursor, ..Default::default()
    })).await
}
```

## Candidate lifecycle

`SubmitCandidate` accepts `scope`, stable client-local `eventId`, `title` (120 Unicode characters),
`content` (2,000), up to eight `sourceRefs` (4 KiB serialized), and sixteen exact
`basedOnRevisionIds`. It returns `{candidateId, status, created, version}`. Retrying identical
arguments with the same event ID returns the existing receipt; changed content with that ID fails.
An unknown write outcome should be retried with the same ID, not a new candidate.

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

There are at most 50 undecided candidates/client and 500 retained items overall. Items and their
idempotency receipts expire 30 days after submission, except unresolved promotion plans. Approval
does not extend candidate retention. Do not use this inbox as an audit log or resend old events
after their retention window.

## Activity lifecycle and read cost

`PublishActivity` accepts `scope`, stable `topicKey` (64 characters), `summary` (180), optional
`expectedVersion`, and `ttlHours` (1–168, default 48). The receipt contains `cardId`, `version`,
`changed`, `expiresAt`. Runtime time supplies timestamps; clients do not supply approximate dates.

Each client keeps at most three topic cards across sessions, not one growing history per chat.
A fourth topic evicts the oldest. An existing card needs its exact version for changed content;
the same content is a no-op and does not renew TTL. Preserve the last receipt in host state or
read own cards before updating. No automatic refresh, periodic model summary, or mandatory
end-of-session write is required. There are at most 192 live cards overall.

`ReadActivity` accepts optional `scopes`, literal topic `query` (120 characters), `limit` (1–5),
`includeOwn` (default false), and a query-local `cursor`. The default empty scopes means all
authorized scopes, not all Store content. The result is bounded to five cards / 900 summary
characters plus identifiers and timestamps. Revoking publishing hides that client's cards.

- `{items, cursor, unchanged:false, replace:true, truncated}` is a new bounded **snapshot**.
  Replace the previous snapshot, including when items is empty after expiry or revocation.
- `{items:[], cursor, unchanged:true}` means retain the prior snapshot.
- Keep cursors per conversation and query. They are content digests, not global watermarks or
  pagination cursors. `truncated` means there are omitted cards; narrow the topic if needed.
- Silence does not mean inactivity; expiry does not mean completion. Read only when recent
  cross-window context could help. Do not poll or treat card text as agent instructions.

There is no model call for storage, TTL, similarity hints or activity reads. Console displays
the current cards; it does not synthesize them into a new authoritative user profile.

## Operational state

The file next to the SQLite Store, `<store stem>.context.json`, holds only these bounded records
and client policies. It is Store-identity-bound, mode 0600, locked across processes and atomically
replaced. The Page database schema is unchanged. Back up this file with the Store if pending
reviews and policies need to survive restoration; do not copy it to a different Store identity.

Expiry is checked on access. Expired content is removed when an authorized read, inspection or
write saves the pruned state; there is no background model job or secure-erasure guarantee.
The lock wait is bounded to two seconds and the operational file to 8 MiB. A busy/error response
is not an empty success. Existing formal Page permissions and responses remain unchanged.
