# Candidate evolution

Candidates are evidence awaiting interpretation, not a queue of independently approvable facts.
Runtime organizes bounded, same-Scope windows after new evidence arrives. A persistent synthesis
records the original candidate versions, chronology, complementary evidence, corrections, conflicts,
maturity rationale, unresolved questions, and zero to four proposed memory outputs. One evolving
subject can produce several memories. Similarity is only a retrieval hint, never truth or permission.

## Ownership and cadence

ContextHub owns candidates, synthesis snapshots, retention, concurrency and review receipts.
The existing maintenance scheduler owns model execution and usage accounting. With maintenance
already enabled, it checks candidate readiness at most every 60 seconds, waits 120 seconds after
new evidence to coalesce submissions, and performs at most one organization job per cycle within
the existing job budget. A bounded window contains at most 20 candidates (16,000 source characters) plus at most six relevant
current Pages (12,000 characters). Up to four bounded previous interpretations supply continuity;
exact Page comparison snapshots are shared across groups rather than copied in storage. Scope checks precede model work. No unchanged window is repeatedly evaluated;
A changed or removed compared Page reopens affected pending groups. New evidence can revisit deferred
peers, while a deferral alone does not cause repeated inference. Continuous arrivals cannot postpone
a waiting candidate beyond ten minutes. Provider failures are retried after 30 minutes, without deleting evidence. Manual queueing is an
operator-only action and does not itself approve content. Pending or deferred candidates no longer
expire. Resolved operational receipts are retained for 30 days after review; formal Pages remain independent.

## Review and recovery

Luna synthesis alone grants no write authority. The operator sees sources and their timeline,
the interpreted evolution, maturity and unresolved issues, and editable proposed memories. Outputs
may create a Page, revise an exact offered current Revision, or mark evidence as already represented.
Each output names its own candidate evidence; overlapping evidence can support multiple outputs.
Only evidence covered by approved outputs is resolved. Other evidence remains available.

Review validates exact synthesis and candidate versions and exact Page heads, then persists the
approved operator or Sol plan before writes. Each output has a stable idempotency key and a durable receipt. A partial
failure is visible and can resume only the identical plan; committed outputs are not written twice.
An operator may explicitly stop an unfinishable submission: no Page is rolled back, confirmed receipts
remain visible, and covered evidence returns to organization. Unknown output outcomes are flagged
for comparison with existing Pages before another approval, rather than assumed unwritten.
Models use local c1/r1 handles; Runtime strictly maps them back to the offered canonical identities.
A structurally rejected model result waits for changed evidence or an explicit retry instead of
spending tokens on the same failed window every timer tick. Provider outages retain a timed retry.
Existing source references and Page provenance are preserved. Untrusted model text is rendered as text.
Synthesis snapshots are superseded when their evidence changes; rejected/deferred evidence is not
silently reintroduced. No global cross-Scope consolidation or extra MCP write authority is added.


## Shared-budget automatic review

When candidate automatic review is enabled in Console, maintenance is in apply mode, and its Infer worker has the upgraded review budget enabled,
settled synthesis outputs enter the same rolling Sol calls/tokens ledger as other maintenance.
There is no additional token allowance and no Astra fallback for candidate writes. The scheduler
alternates priority between existing maintenance and candidate review, including one-job cycles.
At most one candidate group is reviewed per cycle. A group waits five minutes after organization
and two minutes after the latest same-Scope intake. Each output receives its own decision; group
maturity alone cannot approve or block its siblings. Empty drafts continue accumulating.

Sol reads original candidate evidence and up to twelve current comparison Pages (64,000 characters,
requiring complete payloads) and may approve, defer for evidence, identify a specific question, or
recommend no change. Action, evidence subset, Scope and exact target stay fixed. A faithful attributed
hypothesis can be retained as a hypothesis. Changed title/content is independently verified in one
additional Sol call, with no repair loop. Malformed results are held without repeated paid calls.
Admission waits retain the exact evidence/request for cached response recovery. A paused or unavailable
budget never falls back to Luna approval or silently forwards ordinary work into an obligatory human queue.

Runtime persists exact evidence, assessed outputs, review request IDs, usage and the approved plan
before writing. New same-Scope intake or changed comparison heads invalidate approval before the first
write; in-flight plans instead recover through their durable idempotency keys. Pending evidence shared
with an unresolved output or open group question remains pending, without triggering another review
by itself. Source snapshots and review metadata are also attached to formal memory facets so they outlive
the operational inbox receipts. Manual edits to a new plan are not attributed to a previous Sol review.

Existing deployments retain manual review until the operator enables candidate automatic review. Console provides a global pause switch and per-output review explanations. A pause prevents new calls
and writes, including application of a result that arrives after the pause; already completed writes stay
visible. An operator can withdraw an exact automatic output: a newly created Page is archived without
deleting its content, while an update is reversed through a new Revision restoring the original content
and metadata. Later edits cause a conflict rather than being overwritten. Undo has its own persisted
intent and receipts. Existing Page editing remains available for corrections. Routine successes stay in
the history, while missing information is described as a specific question.
