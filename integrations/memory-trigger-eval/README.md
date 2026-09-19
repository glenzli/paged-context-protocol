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
