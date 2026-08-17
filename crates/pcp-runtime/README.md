# PCP Runtime

`pcp-runtime` composes one Identity-bound Store, local RPC endpoints,
user-approved client enrollment, and background maintenance. It is the official
service implementation; the normative data and behavior contract remains in the
top-level protocol specification.

It also advertises the versioned, aggregate-only
`pcp.runtime.observer@20260810.1` protocol through Infra Discovery, as defined in
[`OBSERVER.md`](OBSERVER.md). The observer has its own owner-only Unix socket. Its
application protocol does not reuse Console HTTP DTOs or expose PCP content.

Runtime also advertises `pcp.runtime.enrollment@20260810.1` on the same public
Infra Discovery endpoint. A client can request a Principal and Scope set, wait
for approval in PCP Console, and then receive a generation-specific PCP RPC
endpoint. The canonical wire contract and staged Symbiont migration are in
[`ENROLLMENT.md`](ENROLLMENT.md). Static configured endpoints remain available
during migration.

## Boundary

| Layer | Owns | Does not own |
| --- | --- | --- |
| Tenant | Source events, source-local deterministic structure, Page kind, SourceRefs, external-media custody | Global relation graph, maintenance cadence, cross-tenant policy |
| PCP protocol and Store | Identity boundary, stable Pages, immutable Revisions, Relations, authorization, exact reads, atomic commits | A fixed inference model or active prompt |
| Runtime maintainer | Identity-wide candidate discovery, bounded jobs, budgets, timeout, cooldown, worker invocation, validation, commit authority, maintenance ledger | Tenant product behavior or media-byte custody |
| Inference worker | Summary, ordered packing-candidate, relation, validity, and milestone judgments requested by Runtime | Direct Store writes, content packing, Page-head advancement, scheduling, or GC policy |

The maintainer is disabled unless `[maintenance]` is present with
`enabled = true`. It never falls back to automatic similarity-based merging.
Destructive sealed-Page packing has a second opt-in and remains disabled unless
`maintenance.packing.enabled = true`.
Semantic relation maintenance is independently disabled unless
`maintenance.relation.enabled = true`.
`mode = "observe"` is the default and gives the maintainer a read-only PCP
Principal: the worker still evaluates candidates, but Runtime cannot write an
assessment, Summary, or replacement Page. `mode = "apply"` must be selected
explicitly after the worker has been observed against the target Identity.
`initial_delay_seconds` optionally delays the first inventory heartbeat after
process start. `interval_seconds` is only the maximum delay before Runtime
notices another Page-head write; it does not itself authorize a model call.
Runtime persists a write watermark, groups changed Pages by source stream, and
runs semantic maintenance only when `[maintenance.write_trigger]` has both
enough new Pages and a completed quiet period (or its bounded maximum wait).
The first heartbeat establishes a baseline rather than treating an existing
Store backlog as newly written work.

Scheduled packing and Summary work may apply in `mode = "apply"`. Scheduled
semantic relations remain proposals even in apply mode: they are recorded for
review and never enter the asserted relation graph or retrieval path merely
because a worker selected a pair. Direct operator review remains the only path
that asserts a semantic `related_to` edge.

```toml
[maintenance.write_trigger]
# A source stream must receive this many new Pages before normal execution.
min_new_pages = 8
# Give an active writer time to finish the local episode.
quiet_period_seconds = 600
# Bound latency if a stream remains active indefinitely.
max_wait_seconds = 3600
```

## Maintenance Cycle

One cycle is bounded by `max_jobs_per_cycle`:

1. Runtime reads the complete authorized current-Page inventory with a bounded routing excerpt per Page.
2. Runtime deterministically forms a bounded analysis window of sealed leaves and packed anchors that share Scope, kind, and a contiguous SourceSpan, then sends compact head-and-tail routing text as `select_packing`. `analysis_window_pages` controls what the worker can compare; it is independent of the smaller `max_pages` commit limit.
3. The worker may select one ordered coherent episode from that exact window. Lossless packing does not require every Page to state the same fact: questions, answers, corrections, qualifications, and short reasoning transitions may stay together. It does not generate packed content.
4. Runtime validates the selected IDs, aggregate input size, and at most one packed anchor and, in apply mode, calls `pack_pages`; Store rechecks exact heads, source continuity, identity pins, anchor count, retention, and transaction invariants. It then reloads the current-Page inventory before the next phase.
5. A long unsummarized Page may be sent to the worker as `summarize_page`. Runtime reloads the inventory after a Summary write before relation work.
6. Runtime may send overlapping bounded current-Page routing windows as `select_relation`. The request lists already related or previously proposed pairs; the worker can return only two other offered Page IDs. Runtime fixes the relation to symmetric `related_to`, binds the exact current Revisions as basis, rejects stale or excluded pairs, and owns the commit. A window remains active until the worker returns `no_candidate`.
7. Runtime obtains a bounded dry-run retention plan and may ask the worker whether any actual old candidate Revision is a semantic milestone.
8. In apply mode only, Summary writes, packing, Relations, or finite retention leases cross into the PCP commit API. Leases additionally require `maintenance.retention.write_leases = true`. Lease selection and physical collection remain separate operations; the current maintainer does not collect Revision payloads automatically.

Runtime keeps cooldown decisions in `state_path`. This operational state is not
written as user memory. Successful Summary writes remain traceable through normal
PCP provenance and Relations; successful packs preserve each leaf record inside
the flat packed payload and leave a content-free replacement ledger. A packed Page
may later absorb adjacent leaves through a new Revision without changing its Page ID.
A `no_candidate` response also cools down
the exact routing-window Page ID set, so an unchanged Store does not repeatedly
consume semantic-worker tokens.

For a bounded migration or operator-reviewed rebuild, use `maintenance run-batch`.
It persists the normal cooldown ledger, stops when its worker-call budget is
exhausted or no eligible work remains, records one redacted audit entry, and
requires the exact Store identity even in observe mode:

```text
pcp-runtime maintenance run-batch \
  --config /path/to/runtime.toml \
  --mode observe \
  --max-jobs 20 \
  --confirm-identity idn_... \
  --reason "v0.8 semantic migration review"
```

Change `--mode` to `apply` only after reviewing observe results. The batch runner
does not alter the long-running scheduler configuration.

## Worker Contract

`maintenance.worker.provider = "infer_runtime"` uses the official Infer Runtime
Consumer SDK and an independent `pcp-runtime` managed credential. Runtime
discovers Infer locally and submits only `text.summarize` or `reasoning.solve`.
Summary requests are fixed to the configured local deployment with local-only,
offline execution. Packing and Retention judgments use the configured reasoning
deployment; Relation uses it too unless `relation_deployment_id` explicitly
names a higher review deployment. Advanced judgments use a subscription
deployment with an advanced capability floor and medium reasoning effort. Both
paths use unary Responses with background scheduling
priority, a named deployment, zero estimated cost, and no fallback: an
unavailable deployment fails instead of silently selecting a different model.
The defaults are local Qwen 3.5 4B and Luna. Terra can be configured only as an
explicit Relation escalation; it is not an automatic fallback, and Sol has no
maintenance path. The configured output-token limit is sent to the
local Summary deployment. Subscription reasoning omits that unsupported hard
constraint and remains bounded by the strict decision schema and 1 MiB wire
limit. Runtime accepts only strict JSON from `output_text`; missing text,
markdown fences, unknown fields, terminal failure, timeout, or invalid PCP
decisions fail closed.

`maintenance.worker.provider = "command"` executes `program` directly without
a shell. Runtime writes one JSON request to stdin and expects one JSON response
on stdout. stderr is used only for failure diagnostics. The process is killed
on timeout, and responses larger than 1 MiB are rejected.

The worker receives authorized Page content for semantic judgment and therefore
belongs inside the same trust boundary as the configured Host. Enabling an
arbitrary executable or remote-forwarding adapter is an explicit data-access
decision; Runtime does not redact Detail after the configured character budget.

Routing selection receives compact text rather than complete Page payloads:

```json
{
  "operation": "select_packing",
  "pages": [
    {
      "pageId": "pg_...",
      "createdAt": "2026-08-15T08:00:00Z",
      "observedAt": "2026-08-15T07:59:58Z",
      "routingText": "bounded excerpt"
    }
  ],
  "excluded_candidate_sets": []
}
```

The worker may decline to select anything:

```json
{"decision":"no_candidate"}
```

The positive response contains only an ordered subset of Page IDs from the offered
window:

```json
{
  "decision": "candidate",
  "pageIds": ["pg_41", "pg_42", "pg_43"]
}
```

Runtime maps those IDs to the exact offered Revision IDs. Store constructs the
packed payload itself, preserving input payloads, actors, times, SourceRefs,
facets, and provenance without synthesis. Actor identity, authorization,
idempotency, source continuity, reference safety, and atomic replacement are
filled or validated by Runtime and Store. `no_candidate` and `defer` remain
non-Page operational decisions.

Relation selection uses the same compact routing principle but grants less
authority than the Core relation API. A positive decision is exactly:

```json
{"decision":"relate","page_ids":["pg_41","pg_92"]}
```

The worker cannot choose a relation vocabulary, basis Revision, actor, Scope, or
write mode. It should select a pair only when both Pages directly help understand,
verify, or act on the same stable subject or evidence chain. Temporal adjacency,
shared Scope, co-retrieval, lexical similarity, and broad analogies such as both
discussing AI infrastructure or workspaces are not sufficient. When packing is
enabled, unpacked sealed stream leaves are kept out of the relation window so a
premature Relation cannot block lossless packing.
