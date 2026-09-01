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
- any additional corrective evidence in `evidenceRevisionIds`, especially a new Page written after the challenged response; never mislabel it as `usedRevisionIds`;
- the correction or disagreement with the user's meaning and qualifications preserved, not a verbatim copy of their tool-use request;
- enough context to distinguish a factual correction from a preference change or a one-off exception.

The MCP server still prompts for approval before feedback is written. This Skill does not grant write authority.

### New information and replacement

Write independent new information normally, in your own writable Scope. It does not need to claim that an older Page is wrong. Runtime can propose a content update after comparing related evidence, but a newer timestamp, shared subject, or provenance link is not proof of replacement.

When the user explicitly requests a correction:

1. Read the exact old Revision and confirm the correction's meaning, subject, and scope.
2. If the correction deserves its own durable Page, capture that new information with the normal approval. Otherwise put the correction directly in feedback; creating a second Page is optional.
3. Submit feedback in your writable `scope`, referencing the old Revision in `challengedRevisionIds` and any new Page's returned Revision in `evidenceRevisionIds`. `usedRevisionIds` contains only evidence actually used in the challenged response; omit it when no such response is identified.
4. Report that the feedback was recorded and that any replacement/retraction awaits Console approval. Do not say the old Page was revised, replaced, or deleted.

Reading another Scope is enough to challenge its evidence; writing or assessing that Scope is not required. It does not grant permission to publish a derived copy across Scopes. If a new Page genuinely derives from PCP evidence, keep truthful `basedOnRevisionIds` and obtain the required derivation authorization rather than dropping provenance to bypass a denial. A feedback reference is not a provenance-free route for copying private content.

The ordinary plugin does not use `pcp_assess_validity`, privileged Page revision, or direct Relation writes to force a replacement. Console shows the exact old content, new evidence, and Scopes for review. Partial corrections should qualify a claim or record a dispute, not discard unrelated useful content. Approval does not enlarge any client's read permissions or rewrite all downstream summaries.

## Capture sparingly

Use `pcp_capture` only when the user explicitly asks to retain something or when all of these are true:

- the information is confirmed rather than speculative;
- it is likely to matter across future tasks;
- it is not already available from a more authoritative, inexpensive source;
- it can be expressed as one self-contained subject with a clear retention reason.

Good candidates include stable preferences, durable constraints, confirmed decisions, verified findings, and reusable completed outcomes. Preserve the user's language where practical. When the new Page derives from PCP material, include the exact basis Revision IDs so provenance and relations can be established.

Do not capture routine progress, raw transcripts or logs, temporary runtime state, secrets, unverified hypotheses, duplicate summaries, or facts that are cheaply recoverable from the repository. Do not convert an entire task into memory by default.

The MCP server prompts before capture. Present what will be retained and why; never imply that reading PCP authorizes writing to it.

## Compose the stored content

Store the subject, not the act of remembering it. This applies to both capture and feedback:

- **Title and content:** the self-contained fact, preference, decision, constraint or finding. Preserve who holds a view, its scope, uncertainty and meaningful effective dates. Feedback states what is disputed, the correction or disagreement and its grounds; preserve an explicit user request to withdraw a claim without saying withdrawal has already happened.
- **Metadata:** put future utility in `retentionRationale`, known source pointers in `sourceRefs`, actual derivation evidence in `basedOnRevisionIds`, and feedback references in their respective Revision ID fields. Use `observedAt` for a known observation time, not a save confirmation. Do not invent provenance.
- **Current conversation only:** permission to save, confirmation of saving, tool calls, next steps and assistant-authored Console instructions. Do not append “用户要求记录”, “已于某日确认保存”, or “下一步调用 …” to stored content. `explicit_instruction` identifies why capture was authorized; it does not mean storing the save instruction.

Distinguish durable instructions from transient operations by meaning, not keywords. An ongoing preference or project constraint belongs in content; “call MCP now” does not. Permission to retain something is not confirmation that its claims are true. Keep a fact-effective date when it changes the claim, but do not turn today's save date into an event or source date.

Examples (retain the user's language in actual writes):

- User: “记住，以后技术解释用中文，代码标识符保留英文。” Content: “用户偏好技术解释使用中文，代码标识符保留英文。” Not: “用户要求记录此偏好，已确认保存；以后请读取 PCP。”
- User: “图标已经改用不透明背景，帮我记下来。” Content: “Shadow 图标采用不透明背景。” Not: “用户要求记住 Shadow 图标，并于 2026-09-02 确认保存。” Put a known design source in `sourceRefs`, not an invented confirmation sentence.
- User: “旧记录说图标透明，这不对，现在是不透明背景。” Feedback content: “旧记录将 Shadow 图标背景描述为透明；当前采用不透明背景。” Reference the old Revision separately. Do not add “请维护程序替换旧页面并通知用户” or claim the replacement is applied.
- User: “从 9 月 1 日开始改用新方案，今天帮我存一下。” Preserve the effective date and the actual scheme when known; omit the saving date. If “新方案” is not identifiable from evidence, clarify rather than write a context-dependent fragment.

Before a write, read only the proposed title and body: would a later reader understand the subject without this conversation? Remove saving and workflow narration, check that facts and qualifications still match the evidence, and keep metadata separate. Do not mechanically strip instructions, dates or the word “用户”; that would damage real preferences and corrections. If the remaining subject has no durable value, do not capture it.

## Respect source ownership

PCP stores Pages, Revisions, relations, and provenance references. It does not need to understand every tenant-owned original source or media structure. If a Page points to an external source, use the PCP evidence needed to identify it, then let the tenant or source owner perform any source-specific parsing or rendering.
