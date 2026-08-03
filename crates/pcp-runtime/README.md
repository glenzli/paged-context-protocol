# PCP Runtime

`pcp-runtime` composes a Store, fixed-identity local RPC endpoints, and optional
background maintenance. It is a reference deployment component, not part of the
normative PCP data model.

## Boundary

| Layer | Owns | Does not own |
| --- | --- | --- |
| PCP protocol and Store | Immutable Pages, Relations, Refs, authorization, exact reads, atomic Summary/consolidation commits | Timers, prompts, model routing, retry policy |
| Runtime maintainer | Bounded jobs, Scope-specific Principal, budgets, timeout, cooldown, worker invocation, mechanical request fields | Semantic equivalence or generated content |
| Semantic worker | Whether a Summary is useful, candidate selection, conflict detection, lossy synthesis | Direct Store writes, Ref advancement, Relation creation |

The maintainer is disabled unless `[maintenance]` is present with
`enabled = true`. It never falls back to automatic similarity-based merging.
`mode = "observe"` is the default and gives the maintainer a read-only PCP
Principal: the worker still evaluates candidates, but Runtime cannot write an
assessment, Summary, or replacement Page. `mode = "apply"` must be selected
explicitly after the worker has been observed in the Host application.
`initial_delay_seconds` optionally delays the first cycle after process start;
Hosts that restart during development can use it to avoid immediate model calls
and to let dependent services become healthy. Later cycles use
`interval_seconds`.

## Maintenance Cycle

One cycle is bounded by `max_jobs_per_cycle`:

1. Runtime reads a bounded current-Page inventory.
2. A long unsummarized Page may be sent to the worker as `summarize_page`.
3. A bounded routing window may be sent as `select_consolidation`.
4. Runtime validates the selected IDs and reads their exact Detail.
5. The worker returns `consolidate`, `keep_separate`, or `defer`.
6. In apply mode only, `write_summary` or `consolidate` crosses into the PCP commit API.

Runtime keeps cooldown decisions in `state_path`. This operational state is not
written as user memory. Successful Page writes remain fully traceable through
normal PCP provenance and Relations. A `no_candidate` response also cools down
the exact routing-window Page ID set, so an unchanged Store does not repeatedly
consume semantic-worker tokens.

## Worker Contract

`maintenance.worker.program` is executed directly without a shell. Runtime
writes one JSON request to stdin and expects one JSON response on stdout. stderr
is used only for failure diagnostics. The process is killed on timeout, and
responses larger than 1 MiB are rejected.

The worker receives authorized Page content for semantic judgment and therefore
belongs inside the same trust boundary as the configured Host. Enabling an
arbitrary executable or remote-forwarding adapter is an explicit data-access
decision; Runtime does not redact Detail after the configured character budget.

Routing selection receives compact text rather than complete Page payloads:

```json
{
  "operation": "select_consolidation",
  "pages": [
    {
      "refId": "ref_...",
      "pageId": "pg_...",
      "namespace": "project:example",
      "contentChars": 1800,
      "routingText": "bounded Summary or excerpt"
    }
  ],
  "maxPages": 8,
  "excludedCandidateSets": []
}
```

The worker may decline to select anything:

```json
{"decision":"no_candidate","reason":"No Pages express one durable fact."}
```

After selection, Runtime sends exact bounded Detail using
`{"operation":"consolidate_pages", ...}`. A positive decision contains only
semantic output:

```json
{
  "decision": "consolidate",
  "canonicalPageId": "pg_...",
  "content": "A self-contained canonical Page."
}
```

Actor identity, Scope, lifecycle, facets, source references, provenance,
idempotency, `supersedes` Relations, and Ref updates are filled or validated by
Runtime and Store. `keep_separate` and `defer` remain non-Page operational
decisions.
