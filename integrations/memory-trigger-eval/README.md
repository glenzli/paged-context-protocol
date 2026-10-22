# Memory trigger routing replay

This small, provider-independent evaluation checks whether PCP guidance selects the appropriate
memory, activity or retrieval route at a conversation checkpoint. The 24 synthetic cases cover
implicit retention, uncertain value, same-subject increments, known receipts, corrections, no-op
tasks, denied writes and offline recovery. They contain no user transcripts or credentials.

Prepare a blind replay using the current MCP source:

```sh
python3 integrations/memory-trigger-eval/evaluate.py prepare --output /tmp/pcp-replay.json
```

Send the file's `prompt` to the model under evaluation without executing its proposed tools.
Save its JSON response, then score it:

```sh
python3 integrations/memory-trigger-eval/evaluate.py score /tmp/pcp-decisions.json
```

For comparison, prepare another replay with `--source /path/to/previous-lib.rs`. Keep provider,
model and sampling settings the same and retain the source hashes, raw responses and usage.
The preparation step excludes golden plans and scoring reasons. The scorer rejects missing,
duplicate or unknown case IDs and reports positive and negative cases separately. Multiple
calls to one tool remain multiple actions, so a three-write plan fails a one-subject merge case.

This is a routing replay with supplied metadata, not a measurement of spontaneous tool discovery,
actual writes, argument fidelity, real conversation opportunity coverage or long-term retrieval
benefit. Review attribution, uncertainty, exact retry arguments and source IDs separately. A
perfect score does not prove that a host will remember to expose or invoke PCP. Production
changes still need MCP wire and Runtime contract checks; do not execute fixtures against the
user's memory Store. The script makes no API calls and adds no runtime dependencies.

## Blind multi-turn continuation probes

`conversation_eval.py` prepares blind multi-turn continuations. The published
`synthetic_conversations.json` contains ten newly authored fictional conversations, with only
synthetic identities and projects. It covers discussion-only first snapshots, changed next steps,
missing historical context, fresh context, small self-contained tasks, stable user preferences,
uncertain project decisions, denied publication, project binding, and missing recent discussion.
It contains no production transcripts or Store records.

The default `integrations/memory-trigger-eval/conversations.json` remains an ignored **local**
dataset. It is never read by the published tests or downloaded by these scripts. Use an explicit
`--fixtures` path for the synthetic suite or a permitted local dataset. Keep private datasets,
prepared prompts and raw model responses local; do not copy production conversations into the
published synthetic file.

The JSON file is a nonempty array with unique `id` values. Each entry supplies `session` facts,
an `input` list of role/content messages, and an `expect` object with `required`, `arguments`,
and optional `allowed`/`absent` fields. Expectations and case IDs remain outside model input.
Use only data you are permitted to evaluate, and never execute proposed tools against the user's
Store. A missing dataset produces a setup error explaining `--fixtures`, rather than a traceback.

Export the actual binary's MCP `initialize.result.instructions` and `tools/list.result.tools`.
`export_catalog.py` performs only `initialize` and `tools/list` over stdio, including catalog
pagination. It inherits the selected launcher's connection environment, does not call tools, and
does not restart a running service:

```sh
python3 integrations/memory-trigger-eval/export_catalog.py \
  --output /tmp/pcp-catalog.before.json -- integrations/chatgpt/launch-pcp-mcp
```

Preserve that catalog and a copy of the **loaded** AGENTS before edits. Build the changed MCP
binary once, then set `PCP_MCP_BINARY` to that binary for a separate catalog export through the
same launcher. Verify the exported schemas rather than inferring them from a version label.
Prepare both runs with identical AGENTS, fixtures, provider, model and reasoning settings:

```sh
python3 integrations/memory-trigger-eval/conversation_eval.py prepare \
  --fixtures integrations/memory-trigger-eval/synthetic_conversations.json \
  --catalog /tmp/pcp-catalog.before.json --agents /path/to/loaded-AGENTS.md \
  --output /tmp/pcp-probes.before.json
```

Each probe contains a Responses-shaped request (`instructions`, `input`, `tools`). Give the model
only that request; add provider/model settings outside the prompt. Do not append a tool-choice,
memory or evaluation reminder. Do not show fixture IDs, expectations or scoring rules, and do
not execute generated tool calls against the user's Store. Synthetic session facts include
permissions, known project identity and fresh receipts, without prescribing an action.

Save the raw response from each independent continuation as:

```json
{"preparedSha256":"<printed by prepare>","responses":[
  {"id":"probe-001","response":{"status":"completed","output":[]}}
]}
```

```sh
python3 integrations/memory-trigger-eval/conversation_eval.py score \
  --fixtures integrations/memory-trigger-eval/synthetic_conversations.json \
  /tmp/pcp-probes.before.json /tmp/pcp-results.before.json
python3 -m unittest discover -s integrations/memory-trigger-eval -p 'test_*.py'
```

Use the same dataset for preparation and scoring. The published tests always use the synthetic
suite, exercise positive and negative outcomes, and do not depend on the ignored local dataset.
The catalog exporter has a fake stdio-server test verifying that it never sends `tools/call`.
These tests prove preparation, result accounting, boundary arguments and metadata transport;
they do not prove that a model invokes PCP more often.

Scoring separates opportunities captured by route, unexpected writes and argument errors. It
checks project routing, known topic/version reuse, first-card version omission and size limits;
it rejects missing, duplicate, failed or stale-input results. Review summary meaning, uncertainty,
project-qualified new topic keys and attribution separately. Each fixture's `expect.review`
lists semantic checks for a human reviewer and never enters model input. No string matcher proves
that a tentative explanation or attribution was preserved.

These are blind **continuation probes**, not a full rolling host conversation: prior turns and
receipts are supplied fixtures, tool results are not dynamically executed, and registration
before/after are separate probes. They can expose conflicting instructions and missed opportunities,
but cannot measure real host tool discovery, actual writes, long-dialogue recall or spontaneous
invocation frequency. Record model/provider/settings and raw outputs; compare old/new catalog
and AGENTS under the same conditions. If both schemas and wording change, the result compares
the complete catalogs; it does not isolate a causal effect from wording alone. Ten probes are
a bounded diagnostic set, not a population estimate of real missed opportunities. Validate
write contracts in an isolated Runtime, and observe ordinary real conversations after refreshing
host metadata. This suite does not implement host turn counters or lifecycle hooks.

This change publishes preparation/scoring tooling and local contract tests. Real provider/model
blind runs have not been completed; it does not claim measured improvements in model behavior.
