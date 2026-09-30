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

`conversation_eval.py` prepares blind multi-turn continuations from a **local** conversation
dataset. This repository publishes the evaluator, tests and usage instructions, not that dataset.
The default `integrations/memory-trigger-eval/conversations.json` is ignored by Git; use
`--fixtures /path/to/local/conversations.json` to select another local file. Keep datasets,
prepared prompts and raw model responses local. Nothing downloads or creates a dataset for you.

The JSON file is a nonempty array with unique `id` values. Each entry supplies `session` facts,
an `input` list of role/content messages, and an `expect` object with `required`, `arguments`,
and optional `allowed`/`absent` fields. Expectations and case IDs remain outside model input.
Use only data you are permitted to evaluate, and never execute proposed tools against the user's
Store. A missing dataset produces a setup error explaining `--fixtures`, rather than a traceback.

Export the actual binary's MCP `initialize.result.instructions` and `tools/list.result.tools`
as `{"instructions":"...","tools":[...]}`. Then prepare with the **loaded** AGENTS guidance:

```sh
python3 integrations/memory-trigger-eval/conversation_eval.py prepare \
  --fixtures /path/to/local/conversations.json \
  --catalog /tmp/pcp-catalog.json --agents /path/to/AGENTS.md --output /tmp/pcp-probes.json
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
  --fixtures /path/to/local/conversations.json /tmp/pcp-probes.json /tmp/pcp-results.json
python3 -m unittest discover -s integrations/memory-trigger-eval -p 'test_*.py'
```

Use the same local dataset for preparation and scoring. On a checkout without the default
local dataset, the six conversation replay tests explicitly skip; MCP metadata and CLI setup
tests still run. Place your permitted dataset at the ignored default path to run the replay
tests. Do not commit datasets to make skipped tests pass.

Scoring separates activity opportunities captured, unexpected writes and argument errors. It
checks project routing, known topic/version reuse, first-card version omission and size limits;
it rejects missing, duplicate, failed or stale-input results. Review summary meaning, uncertainty,
project-qualified new topic keys and attribution separately. No string matcher proves semantics.

These are blind **continuation probes**, not a full rolling host conversation: prior turns and
receipts are supplied fixtures, tool results are not dynamically executed, and registration
before/after are separate probes. They can expose conflicting instructions and missed opportunities,
but cannot measure real host tool discovery, actual writes, long-dialogue recall or spontaneous
invocation frequency. Record model/provider/settings and raw outputs; compare old/new catalog
and AGENTS under the same conditions before attributing an improvement to guidance. Validate
write contracts in an isolated Runtime, and observe ordinary real conversations after refreshing
host metadata. This suite does not implement host turn counters or lifecycle hooks.

This change publishes preparation/scoring tooling and local contract tests. Real provider/model
blind runs have not been completed; it does not claim measured improvements in model behavior.
