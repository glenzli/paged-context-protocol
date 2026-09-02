# Observing proactive retrieval

This is a small behavioral check, not a trigger-rate benchmark. Unit tests validate
the MCP wire contract and access boundaries; they cannot establish whether an agent
will choose a tool or use its evidence well.

## Procedure

Use fresh agents with the same ordinary user question and no parent conversation.
Do not tell them to use PCP, supply the expected answer, or expose another run's
output. Permit reads only. Record the actual tool calls and failures, exact evidence
reads, whether the evidence affected the answer, and any unrequested writes.

Check both a question where prior user/project context could change the answer and
a self-contained task mentioning the same project. For example:

- Product direction: should a project expand by adding more automatic actions?
- Translation: translate a supplied sentence about adding one automatic action;
  do not add background.

For repeatable comparisons, fix the question, model, effort, tool catalog, available
Store evidence, and context sources. Verify which skill and MCP descriptions each
agent actually received. Run each case multiple times before estimating a rate.
Success is useful, supported recall, not a higher number of tool calls.

## Local observation, 2026-09-03

- Baseline `recall_baseline` loaded cached skill 0.1.1. Without an explicit recall
  request, it called semantic search, identity lookup, and one batch exact-revision
  read: three successful PCP calls, two selected Revisions, no retries or writes.
  Its product recommendation used the retrieved project direction and distinguished
  that direction from implemented behavior. The baseline already triggered recall.
- Same-task child `recall_candidate`, created after installing 0.1.2, still received
  the old 0.1.1 skill path and an obsolete tool namespace. The old path was missing.
  It recovered through another available tool namespace, searched and read the same
  evidence, with five attempted PCP calls including two failures. One failure was
  an unsupported `view` value. This is a mixed-environment observation, **not** a
  clean test of the new skill or evidence of improved invocation.
- Self-contained translation child `recall_negative` used no tools and added no
  project background. This checks a negative case but does not by itself establish
  which updated instructions reached the agent.
- An independent CLI run interpreted the wrapper's "do not read other task history"
  as excluding PCP. It used repository evidence and made no PCP calls. This is
  retained as a wording-confounded observation, not counted as a clean comparison.
- A second independent CLI run received only the ordinary product question,
  read-only constraints, and a request for a source audit. Its native JSONL trace
  confirmed reading `pcp/0.1.2/skills/use-pcp/SKILL.md`, then successfully calling
  `pcp_semantic_search` and `pcp_read_pages` before inspecting the current repository.
  No prompt told it to query PCP or supplied the expected product direction.
  It completed with two successful PCP calls, no PCP retries or writes, and cited
  exact revision evidence while separating its recommendation from verified
  implementation. Subsequent repository inspection was extensive for a brief
  question: bounded PCP calls did not establish lower total task cost or latency.

The independent CLI runs and app subagents are different harnesses; model and
context equivalence were not established. These observations show working
proactive recall and an appropriate negative case, not a measured improvement
over the old policy. Do not discard the stale-environment or wording-confounded
runs when interpreting the result.

Reinstalling a plugin does not refresh the instruction snapshot already held by a
running parent task. A genuinely fresh client process is needed to verify the
installed guidance; a new child of an old task may retain stale discovery metadata.

The updated local plugin is 0.1.2. Its installed skill matched the source SHA-256
`3f909a2a9e45724542f6577616c3cc0c0c70472f824676a97db8b5780d927c96`.
The installed MCP matched the validated release binary SHA-256
`ca40b81d1c414650c011952298f9aaf6b2b5b8ac92af8085dce9368286d197ae`.
An actual installed-plugin `initialize` and `tools/list` probe returned the
read-first server instructions, proactive semantic-search description, and
`readOnlyHint: true`; it did not read or write any Page.
The ChatGPT tunnel was restarted and reported ready. This establishes local
deployment, not selection behavior in a fresh ChatGPT conversation.

Do not commit private Page bodies, user identities, credentials, or full agent
transcripts with the observation. Keep exact live evidence in the authorized local
run. No Store content was written or changed for these probes.
