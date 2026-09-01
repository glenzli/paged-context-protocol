---
name: use-pcp
description: Use PCP to retrieve durable cross-task context, inspect exact revision evidence, submit explicit recall feedback, or selectively capture confirmed reusable context. Use when prior decisions, preferences, constraints, findings, or user-requested retention matter; do not use for routine task state or ordinary repository facts.
---

# Use PCP

Treat PCP as durable context with provenance, not as a transcript store or a mandatory preflight for every task.

## Decide whether PCP applies

Use PCP when the task plausibly depends on information that may outlive one conversation:

- a prior decision, user preference, or stable constraint;
- an earlier verified finding or completed outcome;
- context shared across projects, tools, or tasks;
- an explicit request to recall, search, retain, or correct PCP context.

Skip PCP when the current request and workspace already provide enough evidence, when the fact is cheap to recover from authoritative source files, or when only temporary progress is involved. Do not query PCP merely because its tools are available.

## Retrieve conservatively

1. Call `pcp_whoami` when identity, scopes, or cross-scope access matters. Do not infer access from the plugin configuration.
2. Start with `pcp_semantic_search` for meaning-based recall. Use `pcp_search_pages` for exact text or known identifiers and `pcp_browse_index` for bounded browsing.
3. Read the exact selected revisions with `pcp_read_pages` before relying on their contents. A Page is the stable identity; a Revision is the exact evidence and provenance.
4. Use `pcp_match_intent` only when Router-assisted intent matching is worth its extra analysis. Use `pcp_expand_graph` only after selecting an anchor whose relations are relevant.
5. Prefer a small, well-supported result set. Report uncertainty and conflicts instead of blending revisions into an unsupported memory.

Do not treat a search hit, summary, relation, or older revision as current truth without checking its revision evidence and the authoritative workspace or external source when available.

## Handle challenged recall explicitly

When the user rejects, corrects, or materially narrows recalled context, do not silently rewrite or delete history. Use `pcp_submit_feedback` only after confirming the user's intent, and include:

- the exact Revision IDs whose content was challenged;
- the exact Revision IDs actually used in the response, when different;
- the correction or disagreement in the user's own terms;
- enough context to distinguish a factual correction from a preference change or a one-off exception.

The MCP server still prompts for approval before feedback is written. This Skill does not grant write authority.

## Capture sparingly

Use `pcp_capture` only when the user explicitly asks to retain something or when all of these are true:

- the information is confirmed rather than speculative;
- it is likely to matter across future tasks;
- it is not already available from a more authoritative, inexpensive source;
- it can be expressed as one self-contained subject with a clear retention reason.

Good candidates include stable preferences, durable constraints, confirmed decisions, verified findings, and reusable completed outcomes. Preserve the user's language where practical. When the new Page derives from PCP material, include the exact basis Revision IDs so provenance and relations can be established.

Do not capture routine progress, raw transcripts or logs, temporary runtime state, secrets, unverified hypotheses, duplicate summaries, or facts that are cheaply recoverable from the repository. Do not convert an entire task into memory by default.

The MCP server prompts before capture. Present what will be retained and why; never imply that reading PCP authorizes writing to it.

## Respect source ownership

PCP stores Pages, Revisions, relations, and provenance references. It does not need to understand every tenant-owned original source or media structure. If a Page points to an external source, use the PCP evidence needed to identify it, then let the tenant or source owner perform any source-specific parsing or rendering.
