# Experience memory foundation

PCP retains useful attempts and observed outcomes so later tasks can revisit an explanation.
An observation, its interpretation, and an executable strategy have different meanings. This
extension does not turn a retained experience into a Skill, generate a Tool, or authorize execution.

## Ownership and compatibility

The existing MCP capture/candidate tools accept concise experience accounts in ordinary prose.
There are no new MCP tools, required fields, per-turn checks, or reflection calls. A useful account
keeps the applicable conditions, attempt, observation, tentative explanation and unresolved point
when known; missing fields are not an invitation to invent them.

Native hosts may use `pcp_client::experience::Experience` and the Runtime-local
`ContextHubRequest::SubmitExperience`. The existing `CandidateInput` source and wire shape are
unchanged. `runtime_experience_memory_v1` advertises the extension. Experience delivery uses the
same per-principal candidate opt-in, Scope ACL, quota, review and maintenance budget. A host cannot
select its authenticated submitting identity through the payload.

`topicKey` is optional and producer-local. Runtime compares it only within the same principal and
Scope. It prioritizes related evidence in the existing bounded organizer; it is not a global ID,
proof of relevance, or independent corroboration. A new outcome is a new immutable event with the
same topic key. Exact retransmissions reuse their event identity and do not wake another cycle.

ContextHub owns the optional structured evidence, candidate versions and review atomicity. An
experience contains an attempt and observation; conditions, interpretation, up to four questions
and up to four execution receipts are optional. Metadata is bounded to 12,000 UTF-8 bytes and 6,000
characters and counts against the existing 8,000-character organization window. Raw logs are not
copied into the model prompt. Observation fields remain producer reports even after formal retention.

Manual and automatic review both preserve original experience snapshots in `experienceEvidence`
facets, including the submitting principal, sources and exact referenced PCP Revisions. They survive
terminal inbox cleanup. Revisions retain previous interpretations; source references alone do not
guarantee that the source host still retains or can resolve the original material.

## Native receipt adapters

`experience::receipts::infer_response` accepts an Infer response and records its ID, model and
execution status. `dev_mesh_commit` accepts a completed coordination direct-commit receipt and
records its exact revision and publication status. Adapters whitelist fields and hash the supplied
receipt bytes. They do not copy prompts, output bodies, credentials or arbitrary validation prose.
The caller supplies the actual archive locator for Dev Mesh; PCP never dereferences it.

Receipt stages distinguish inference, validation, publication and task results. A completed model
response does not establish task success; a publication receipt does not independently verify tests.
Full response adapters include a digest of the supplied source bytes. In-process Infer observations
that only have ID/model/status omit a source digest rather than claim to hash the original response.

The PCP Infer worker now attaches actual execution receipts to organization results automatically.
These are stored separately as `executionReceipts` on the synthesis and `organizationExecutions`
on the retained memory, never mixed with the original experience's supporting receipts. Existing
advanced review steps continue to retain their own request IDs and usage accounting.

External hosts can attach native receipts at their existing meaningful checkpoints using the SDK
or CLI below. This release provides adapters and delivery, not a watcher over every Dev Mesh run
or the full ChatGPT/Codex conversation. The shared tunnel only exposes operations the host sends.

## Offline delivery

`ExperienceOutbox` belongs to the producing host. It uses a private directory, atomic staging and
a single-writer lock, and binds each record to the expected Store identity and principal. The host
can stage an experience while PCP is offline and call `flush_one` at a later native checkpoint.
One invocation sends at most one exact request. It does not create a polling service or a second
model. Capacity is 64 pending records with at most 32,000 bytes per record; full queues retain the
existing records and reject new staging instead of silently dropping evidence.

Offline failures, denials and unknown acknowledgements retain the original request. Hosts must
report actionable rejection and stop retrying on denial; no fallback identity or direct Store write
is attempted. Lost acknowledgements may be reconciled by the same idempotent candidate request.
Records older than seven days are held for receipt comparison, safely inside the Runtime's 30-day
terminal receipt retention. A backwards clock or stale delivery lock also requires inspection.
Pending filenames, locators and client IDs are correlation data, never authorization.

```rust,ignore
use pcp_client::experience::{Experience, outbox::ExperienceOutbox, receipts};
// Use an existing task outcome with clear future value; do not stage every success/log.
let receipt = receipts::infer_response(&serialized_response)?;
experience.receipts.push(receipt);
let pending = ExperienceOutbox::open(private_directory, expected_identity, expected_principal)?;
pending.stage(experience.into_candidate(scope, title)?)?;
// The host handles denial/offline errors; this call never starts a retry loop.
pending.flush_one(approved_runtime_client).await?;
```

CLI equivalents, intended for native integration rather than model checklists:

```sh
pcp experience-receipt infer < response.json
pcp experience-receipt dev-mesh /absolute/archive/commit.json < commit.json
pcp experience-stage /private/host-outbox IDENTITY PRINCIPAL < experience.json
PCP_RUNTIME_SOCKET=/approved/current/socket pcp experience-flush /private/host-outbox IDENTITY PRINCIPAL
```

`experience.json` contains `scope`, `title` and `experience`; the last contains `attempt`,
`observation` and the optional fields above. Stage and receipt conversion work without opening a
Store. Flush requires a Runtime; embedded fallback is refused. Ordinary host integration should
reuse its enrolled client and discovery/reconnect handling rather than persist a generation socket.

## Revisiting evidence

The existing organizer preserves prior explanations and open questions. Fresh same-topic evidence
or changed compared Page revisions can reopen a pending interpretation. Unchanged evidence,
elapsed time alone and duplicate delivery do not trigger repeated model work. Provider cooldowns,
coalescing, complete-evidence review, unresolved evidence retention and shared review budgets remain
in force. This first version does not infer a global learning curriculum or upgrade memory into
executable skills automatically.
