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
`initial_delay_seconds` optionally delays the first cycle after process start;
Deployments that restart during development can use it to avoid immediate model calls
and to let dependent services become healthy. Later cycles use
`interval_seconds`.

## Maintenance Cycle

One cycle is bounded by `max_jobs_per_cycle`:

1. Runtime reads a bounded current-Page inventory.
2. A long unsummarized Page may be sent to the worker as `summarize_page`.
3. Runtime deterministically forms a bounded window of sealed leaves and at most one packed anchor that share Scope, kind, and a contiguous SourceSpan, then sends it as `select_packing`.
4. The worker may select one ordered subset from that exact window. It does not generate packed content.
5. Runtime validates the selected IDs and, in apply mode, calls `pack_pages`; Store rechecks exact heads, source continuity, leaf references, anchor count, retention, and transaction invariants.
6. Runtime may send a bounded current-Page routing window as `select_relation`. The worker can return only two offered Page IDs; Runtime fixes the relation to symmetric `related_to`, binds the exact current Revisions as basis, rejects stale or existing pairs, and owns the commit.
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

## Worker Contract

`maintenance.worker.provider = "infer_runtime"` uses the official Infer Runtime
Consumer SDK and an independent `pcp-runtime` managed credential. Runtime
discovers Infer locally, submits only `text.summarize` or `reasoning.solve`, and
fixes every request to background, local-only, offline, no-fallback, and zero
cost. It polls the durable response to a terminal state and accepts only strict
JSON from `output_text`; missing text, markdown fences, unknown fields, terminal
failure, timeout, or invalid PCP decisions fail closed. A known background
response is cancelled best-effort on timeout.

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
write mode. It should select a pair only for a substantive semantic connection;
temporal adjacency, shared Scope, co-retrieval, or lexical similarity alone are
not sufficient. When packing is enabled, unpacked sealed stream leaves are kept
out of the relation window so a premature Relation cannot block lossless packing.
