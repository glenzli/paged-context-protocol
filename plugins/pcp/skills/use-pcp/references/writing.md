# Capture and feedback

Read this only when writing durable context or recording a correction.

## Capture threshold

Retain a subject when explicitly requested, or when it is confirmed, reusable across future tasks,
not already represented, and not cheaply recoverable from an authoritative source. Examples include
stable preferences, constraints, decisions, verified findings and reusable completed outcomes.
Do not record routine progress, raw transcripts/logs, secrets, speculation or duplicate summaries.
Permission to retain a claim is not confirmation that the claim is true.

Search for likely duplicates, then inspect relevant exact Revisions. Present the proposed subject
and why it is worth keeping. Tool approval remains required; this Skill grants no write authority.

## Content versus metadata

- Title/body: the standalone fact, preference, decision or finding, in the user's language. Keep
  attribution, uncertainty, scope and meaningful effective dates.
- `retentionRationale`: future utility, not a repeated save acknowledgement.
- `sourceRefs`: known source coordinates. `basedOnRevisionIds`: exact PCP evidence actually used
  to derive the new content, not merely related Pages.
- `observedAt`: known observation time. Omit if unknown; Runtime records creation time. Use RFC 3339
  with an offset for a known instant, or `YYYY-MM-DD` for day precision. Never invent midnight or
  replace an unknown observation date with today's save date.
- Current conversation only: permission to save, tool calls, saving confirmation and assistant
  next steps. Do not append these to the Page. `explicit_instruction` means the user requested
  retention; it does not mean the save instruction is the content.

An ongoing preference such as “技术解释使用中文，代码标识符保留英文” is durable content.
“用户要求记录该偏好，已确认保存，下一步调用 PCP” is workflow narration, not that preference.
“旧记录说透明，当前采用不透明背景” is correction content; “请维护程序替换并通知用户” is not.
Preserve a genuine effective date, such as “自 9 月 1 日起采用新方案”, when the scheme is identifiable.
Do not strip all instructions, dates or mentions of the user mechanically; that damages real meaning.

Before writing, read only the proposed title/body: can a future reader understand the subject without
this conversation, and are pending actions still distinguishable from completed facts?

## Feedback and replacement

Confirm the correction's intent, subject and scope and read the exact old Revision. If new information
deserves a separate durable Page, capture it normally; otherwise the correction can live in feedback.
Independent new information does not need to declare the old Page wrong.

Use `pcp_submit_feedback` with:

- `challengedRevisionIds`: exact old Revisions, not substituted current heads;
- `usedRevisionIds`: only evidence actually used in the challenged response; omit if no response is identified;
- `evidenceRevisionIds`: new corrective evidence, including a newly captured Revision;
- `content`: disagreement/correction, grounds and affected scope; preserve explicit withdrawal intent
  without claiming withdrawal has already been applied;
- `scope`: the writable Scope containing feedback, not necessarily the challenged Page's Scope.

Readable evidence can be challenged across Scopes without modifying those Scopes. Replacements and
retractions await Console approval. Report “feedback recorded/pending review”, not “old Page fixed”.
Do not use privileged validity, Page revision or Relation tools to bypass review.

Read access does not authorize deriving and publishing a copy across Scopes. Keep truthful
`basedOnRevisionIds` and obtain derivation permission instead of dropping provenance after denial.
Feedback is not a way to copy private content without provenance. Partial corrections should not
discard unrelated useful claims. A write timeout has unknown outcome: check returned IDs or the
exact proposed content before retrying.
