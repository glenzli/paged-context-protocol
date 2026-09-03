# PCP Runtime

`pcp-runtime` composes one Identity-bound Store, local RPC endpoints,
user-approved client enrollment, and optional background maintenance. It is the
project-maintained multi-client service implementation and one deployment profile,
not a prerequisite of the Core protocol. An embedded Host may compose the same
Store and client contracts in-process without RPC, Discovery, enrollment, Observer,
or a background scheduler. The normative data and behavior contract remains in the
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
| Tenant / Host | Source events, source-local deterministic structure, Page kind, SourceRefs, explicit feedback against exact recalled Revisions, active-context planning, and external-source custody, parsing, search, and rendering | Data outside its grants, the global relation graph, cross-tenant policy |
| PCP protocol | Identity, Page, Revision, Relation, authorization, provenance, and operation invariants | A daemon, scheduler, fixed inference model, or active prompt |
| Store implementation | Transactions, current-head indexes, authorization enforcement, exact reads, retention roots, and advertised atomic operations | Retrieval timing, active-context assembly, or tenant product behavior |
| Runtime service | Local RPC, injected Principal and AccessSession, enrollment, Discovery, Observer, and optional maintenance coordination | Active-context assembly or external-source parsing and rendering |
| Optional Runtime maintainer | Identity-wide maintenance-item discovery, bounded convergence, typed review queue, budgets, timeout, cooldown, worker invocation, validation, commit authority, maintenance ledger | Tenant product behavior or external-source custody, parsing, search, or rendering |
| Inference worker | Feedback reconciliation, Summary, ordered packing-candidate, relation, Topic, archive-review, and milestone judgments requested by Runtime | Direct Store writes, content packing, Page-head advancement, scheduling, lifecycle mutation, or GC policy |

An embedded deployment combines the Tenant / Host and Store roles in one process.
That composition does not grant access outside the Host's Scopes or make Runtime
control-plane and maintenance behavior part of protocol conformance.

## Local operator access

A same-user local administration surface such as managed PCP Console may use a
Store-wide static endpoint:

```toml
[[endpoints]]
socket_path = "run/pcp-console.sock"
client_id = "operator:local"
client_type = "service"
client_name = "PCP Console"
access_mode = "admin"
store_wide = true
allowed_scopes = []
allow_cross_scope_derivation = true
```

`store_wide = true` is accepted only for an `admin` service Principal. Runtime
records its permissions separately from ordinary Scope grants, and Store
authorization resolves an unscoped request against the current local Scope
inventory. New Scopes therefore become visible without regenerating the
session. Enrollment never issues Store-wide access, and the optional maintainer
continues to use its independently configured `allowed_scopes`.

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
process start. Successful content writes through a Runtime endpoint wake the
maintainer immediately and break any idle backoff; the wake only refreshes the
inventory watermark and does not itself authorize a model call. Runtime
persists changed regions, groups source-backed Pages by stream, and runs
semantic maintenance only when `[maintenance.write_trigger]` has enough new
Pages plus a completed quiet period, or reaches its absolute maximum wait.

With no dirty regions, empty cycles back off exponentially from
`interval_seconds` to `max_interval_seconds`. These timer wakes are safety polls
for writes outside Runtime's observable endpoint path. Productive cycles that
leave follow-up work retry after 30 seconds; failures use an independent bounded
30-second exponential retry. The first heartbeat establishes a baseline rather
than treating an existing Store backlog as newly written work.

```toml
[maintenance]
interval_seconds = 1800
max_interval_seconds = 86400

[maintenance.reconciliation]
enabled = true
discover_updates = true
# Maximum combined detail made available to one bounded feedback decision.
max_input_chars = 32000
# A deferred or rejected proposal is not immediately re-run.
retry_after_seconds = 3600
```

Scheduled packing and Summary work may apply in `mode = "apply"`. A relation is
applied automatically only for the narrow structural case of two continuous
Pack Pages in one source stream with a shared protected identifier. General
relations, Topic Pages, archive recommendations, and high-impact feedback reconciliation enter one persistent typed
review queue even in apply mode. Background maintenance and Console `Run now`
use this same controller and queue; queued items affect neither retrieval nor
lifecycle state before acceptance.

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

1. Runtime selects eligible explicit feedback from a bounded pending window and reads its exact feedback, challenged, used, and additional evidence Revisions. Only a challenged Revision may be the target; `evidenceRevisionIds` can include newer corrective content without claiming the old response used it. Low-impact `qualified` or `disputed` decisions may apply under policy only within one Scope. Cross-Scope decisions, `superseded`, `retracted`, external claims, and uncertain outcomes go to review. Each challenged target resolves independently; pending reviews are not repeatedly analyzed. Rejected feedback proposals may be reconsidered after the configured cooldown; a rejected ordinary-content pair is not re-proposed unchanged.
2. Runtime reads the complete authorized current-Page inventory with a bounded routing excerpt per Page.
3. Runtime deterministically forms a bounded analysis window of sealed leaves and packed anchors that share Scope, kind, and a contiguous SourceSpan, then sends compact head-and-tail routing text as `select_packing`. `analysis_window_pages` controls what the worker can compare; it is independent of the smaller `max_pages` commit limit.
4. The worker may select one ordered coherent episode from that exact window. Lossless packing does not require every Page to state the same fact: questions, answers, corrections, qualifications, and short reasoning transitions may stay together. It does not generate packed content.
5. Runtime validates the selected IDs, aggregate input size, and at most one packed anchor and, in apply mode, calls `pack_pages`; Store rechecks exact heads, source continuity, identity pins, anchor count, retention, and transaction invariants. It then reloads the current-Page inventory before the next phase.
6. With `reconciliation.discover_updates`, ordinary writes may trigger a bounded `review_update` comparison before Summary work. At most 48 recent anchors contribute up to eight exact provenance inputs and two subject-overlap matches each. Older comparison Pages can come from other authorized Scopes. One eligible pair is analyzed per job, using complete bounded content; oversized or incomplete evidence is deferred. This is not an exhaustive semantic duplicate search. A later timestamp, similarity, or provenance never authorizes replacement. All discovered decisions require Console review, including in apply mode; there is no fabricated feedback Page. The operation uses the baseline reasoning route at high effort and is not in the default Sol escalation set.
   A long unsummarized Page may then be sent to the worker as `summarize_page`. Runtime reloads the inventory after a Summary write before relation work.
7. Runtime may send overlapping bounded current-Page routing windows as `select_relation`. Exact current Page pairs connected by provenance inputs are offered before broad recency windows, but provenance never asserts a Relation. The request lists already related or previously reviewed pairs; the worker can return only two other offered Page IDs. Runtime fixes the relation to symmetric `related_to`, binds the exact current Revisions as basis, rejects stale or excluded pairs, and sends general semantic relations to review.
8. After relation work quiesces, Runtime may ask for a source-grounded Topic front door. Valid Topic proposals and conservative archive recommendations are persisted as typed review items; archive is never applied automatically.
9. Runtime obtains a bounded dry-run Revision-payload retention plan and may ask the worker whether an eligible old Revision from that plan is a semantic milestone. These retention candidates are not Runtime context-inbox candidates.
10. In apply mode only, validated low-risk reconciliation, Summary writes, packing, structurally low-risk Relations, or finite retention leases cross into the PCP commit API. Leases additionally require `maintenance.retention.write_leases = true`. Lease selection and physical collection remain separate operations; the current maintainer does not collect Revision payloads automatically.

Runtime keeps cooldown decisions in `state_path`. This operational state is not
written as user memory. Successful Summary writes remain traceable through normal
PCP provenance and Relations; successful packs preserve each leaf record inside
the flat packed payload and leave a content-free replacement ledger. A packed Page
may later absorb adjacent leaves through a new Revision without changing its Page ID.
A `no_candidate` response also cools down
the exact routing-window Page ID set, so an unchanged Store does not repeatedly
consume semantic-worker tokens.

### Correcting content across clients

A contributor can write independent new information in its own Scope, then optionally submit explicit feedback against readable old Revisions. Cross-Scope feedback does not require target-write or derivation permission. Actual cross-Scope derived captures still require derivation permission; feedback is not a way to copy content without it.

Console review presents the exact old content, new evidence, proposed disposition, and involved Scopes. Approval rechecks current Page heads and the Validity head observed during review. A changed target, replacement, or assessment makes the proposal stale rather than overwriting newer work. Replacements and retractions never auto-apply. Cross-Scope approval can record a rationale in the target's Scope but does not grant its readers access to the replacement's Scope.

`apply_reconciliation` remains a privileged API. It accepts an optional `feedbackRevisionId` for explicit feedback, or none for a discovered update, plus `expectedAssessmentRevisionId` and exact `basisRevisionIds`. The result includes `affectedRevisionIds` for dependent material. This transaction does not rewrite every downstream Summary or Topic; those remain separate maintenance decisions.

For the Codex plugin, capture new durable evidence first when it deserves a separate Page, then submit (using real IDs returned by reads and writes):

```json
{
  "scope": "user:your-identity",
  "kind": "correction",
  "content": "The user confirmed the new evidence corrects the earlier claim.",
  "challengedRevisionIds": ["rev_old"],
  "evidenceRevisionIds": ["rev_new"],
  "usedRevisionIds": []
}
```

This is an example of `pcp_submit_feedback`, not an approval or direct revision. Capture is optional if the feedback text itself is sufficient. Use only actually-used context in `usedRevisionIds` and a writable destination `scope`, not necessarily the challenged Page's Scope.

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
Consumer SDK and an independent `pcp-runtime` managed credential. Summary and
reasoning requests default to the explicitly named Luna deployment. There is no
router fallback. `relation_deployment_id` may still pin Relation work to another
baseline deployment, but it is not the uncertainty escalation path.

`escalation_deployment_id` enables one sparse, bounded second opinion. Runtime
uses it only after the baseline returns the explicit `defer` contract for an
operation listed in `escalation_operations`; missing evidence, stale candidates,
invalid output, transport failures, and schema failures do not escalate. The
default eligible set is feedback reconciliation, packing selection/analysis, Relation, Topic, and archive
review. Summary and retention remain Luna-only. A typical policy is:

```toml
[maintenance.worker]
provider = "infer_runtime"
credential_file = "/absolute/path/to/pcp-runtime.token"
summary_deployment_id = "codex_gpt_5_6_luna"
reasoning_deployment_id = "codex_gpt_5_6_luna"
escalation_deployment_id = "codex_gpt_5_6_sol"
```

Both attempts use unary Responses with background scheduling priority, a named
deployment, zero estimated cost, and no fallback: an unavailable deployment
fails instead of silently selecting a different model. Runtime accepts only
strict JSON from `output_text`; missing text,
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
