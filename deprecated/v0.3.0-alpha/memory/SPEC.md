# PCP-Native Memory Profile (P-Mem) - v0.1-draft

> **Deprecated with PCP v0.3.0-alpha.** This profile is preserved as a design
> record. See [../README.md](../README.md) and the current
> [PCP specification](../../../PROTOCOL-en.md).

## 0. Scope

PCP-native Memory, also called Paged Memory or P-Mem, is the same-origin
persistence profile for Paged-Context-Protocol (PCP).

It is not a generic RAG database and not a second protocol beside PCP. Its job is
to persist, retrieve, and maintain PCP Logical Pages outside the current context
window while preserving enough structure for those pages to re-enter the current
Logical Address Space (LAS) without translation loss.

This document refines the earlier `glenzli/paged-memory` design into an internal
profile of this repository.

## 1. Design Goals

- Make current context pages and historical memory pages addressable through the
  same LAS semantics.
- Preserve Page identity across residency changes: `in_context`, `indexed`, and
  `external_memory`.
- Support model-assisted routing over structured page manifests instead of using
  vectors as the final authority.
- Support on-demand evidence resolution through `content_mode` and
  `available_modes`.
- Provide a write-back path for PCP Consolidator artifacts.
- Keep Memory maintenance asynchronous so it does not block the active PCP
  runtime.

## 2. Non-Goals

- P-Mem does not replace PCP's runtime `Linear_Flow`.
- P-Mem does not require every stored page to carry full source content.
- P-Mem does not require every page to support every evidence resolution.
- P-Mem does not treat vector similarity as sufficient authority for logical
  recall.
- P-Mem does not decide application permissions or tool execution policy.

## 3. Core Principle: Same-Origin Page Store

A PCP-native Memory store must preserve the same core page shape used by PCP:

- `id`
- `type`: `Original | Consolidated`
- `trust`: `system | history | audited | sealed`
- `timestamp`
- `summary`
- `keywords`
- `anchors`
- `source_ids`
- `source_ref`
- `source_spans`
- `schema`
- `content_mode`
- `available_modes`
- version and provenance metadata

The physical location of a Page is a residency state, not part of its logical
identity. A Page can move from the active context to Memory, or be recalled from
Memory into the active context, without changing its logical identity.

## 4. Residency Model

P-Mem distinguishes logical identity from residency:

| Residency | Meaning |
| :--- | :--- |
| `in_context` | The Page is currently injected into `<Linear_Flow>`. |
| `indexed` | The Page is known to the active PCP runtime but not fully injected. |
| `external_memory` | The Page is stored outside the active runtime and can be queried or fetched. |
| `archived` | The Page is retained for audit or history but normally excluded from recall. |

Residency changes should not mutate `id`, `source_ids`, `source_ref`,
`source_spans`, or provenance chains.

## 5. Page Record

A minimal Memory Page Record should contain:

```json
{
  "id": "short-hash",
  "type": "Original",
  "trust": "history",
  "timestamp": "2026-06-25T20:11:36+08:00",
  "summary": "Anchor-preserving routing summary.",
  "keywords": ["definition", "compactness"],
  "anchors": ["Definition 2.1", "X", "finite subcover"],
  "source_ids": [],
  "source_ref": "file://book/ch02.md",
  "source_spans": ["L40-L65"],
  "schema": "reasoning_chain",
  "content_mode": "AnchoredSummary",
  "available_modes": ["SummaryOnly", "AnchoredSummary", "Excerpt", "Full"],
  "residency": "external_memory",
  "version": "1",
  "provenance": [
    {
      "event": "write_back",
      "source": "pcp-consolidator",
      "timestamp": "2026-06-25T20:11:36+08:00"
    }
  ]
}
```

For `Consolidated` Pages, `source_ids` should point to child `Original` or
`Consolidated` Pages. A `Consolidated` Page may be recalled as a routing
container and later unpacked through `Consult`.

## 6. Evidence Resolution

P-Mem must keep the current payload separate from available payloads:

- `content_mode`: what is actually returned now.
- `available_modes`: what can be requested later.

Valid resolution levels:

| Mode | Meaning |
| :--- | :--- |
| `SummaryOnly` | Routing summary only. Not final evidence. |
| `AnchoredSummary` | Summary plus anchors and source locations. |
| `Excerpt` | Intent-aligned local evidence spans. |
| `Full` | Complete source payload, only when required and affordable. |

The Memory store should not materialize `Full` by default. It should return the
lowest resolution that is sufficient for the request, unless the caller
explicitly asks for a higher resolution and the Host budget allows it.

## 7. Interfaces

This section defines logical contracts, not transport details. JSON-RPC, local
function calls, HTTP, SQLite functions, or CLI wrappers are all acceptable if
they preserve the semantics.

### 7.1 QueryMemory

Purpose: recall candidate pages by complete Intent Focus.

Input:

```json
{
  "intent_focus": "Need definitions and prior lemmas for proving compactness is preserved by finite products.",
  "constraints": {
    "schema": "reasoning_chain",
    "trust_min": "history",
    "max_pages": 12,
    "desired_content_mode": "AnchoredSummary"
  }
}
```

Output:

```json
{
  "pages": [],
  "routing_trace": {
    "candidate_count": 84,
    "selected_count": 12,
    "desired_content_mode": "AnchoredSummary"
  }
}
```

Requirements:

- The query must accept the full Intent Focus, not only extracted keywords.
- Keyword, graph, timestamp, and vector filters may be used for candidate
  narrowing.
- A model-assisted or rule-constrained Router should make the final relevance
  decision from structured manifests.
- Returned pages must state actual `content_mode` and `available_modes`.

### 7.2 FetchMemory

Purpose: fetch a known page or source span at a requested resolution.

Input:

```json
{
  "id": "short-hash",
  "content_mode": "Excerpt",
  "span_hint": "definition of compactness and finite subcover condition"
}
```

Alternative locators:

```json
{
  "source_ref": "file://book/ch02.md",
  "source_spans": ["L40-L65"],
  "content_mode": "Full"
}
```

Output:

```json
{
  "page": {
    "id": "short-hash",
    "type": "Original",
    "content_mode": "Excerpt",
    "available_modes": ["SummaryOnly", "AnchoredSummary", "Excerpt", "Full"],
    "excerpt": "..."
  },
  "downgrade_reason": null
}
```

If the requested resolution is unavailable or over budget, Memory should return
the highest available resolution that does not exceed the request and provide a
`downgrade_reason`.

### 7.3 WritePages

Purpose: accept PCP write-back artifacts.

Input should include one or more Page Records produced by the PCP Consolidator
or an Adapter. P-Mem must preserve provenance and version metadata.

Write behavior:

- New IDs are inserted.
- Existing IDs create a new version unless the write is explicitly idempotent.
- `trust="sealed"` should only be preserved if source trust and provenance are
  compatible with PCP's trust rules.
- Writes from non-native adapters should normally enter as `trust="audited"` or
  lower application-defined trust.

### 7.4 MarkFeedback

Purpose: record retrieval feedback.

Feedback has two scopes:

| Scope | Meaning |
| :--- | :--- |
| `session` | Current task says this page is irrelevant or noisy. Equivalent to PCP `Purge` negative feedback in the active topic. |
| `durable` | The memory itself is obsolete, false, superseded, or unsafe. Requires explicit caller intent and should be audited. |

PCP `Purge` should not automatically become durable deletion. A page irrelevant
to one task may remain valuable elsewhere.

## 8. Memory Router

The Memory Router performs recall within persistent pages. It should use a
two-stage pattern compatible with PCP:

1. Candidate narrowing over keywords, graph edges, timestamps, schemas, or
   optional vector indexes.
2. Precision selection by comparing full Intent Focus against page summaries,
   anchors, source references, and dependency metadata.

Vectors are allowed as cheap candidate filters, but they must not be the final
authority for complex semantic or mathematical relevance.

The Router should output:

- selected Page IDs
- relevance reasons
- suggested `desired_content_mode`
- possible missing evidence that may require `FetchMemory`

## 9. Memory Consolidator

The Memory Consolidator is an asynchronous maintenance component. It may:

- merge related pages into new `Consolidated` Pages;
- add or improve anchors and source spans;
- detect duplicate pages;
- update topic roots and schema assignments;
- mark stale, superseded, or low-value pages for lower recall priority;
- preserve rejected paths and important negative results.

The Consolidator should not overwrite source pages destructively. It should
create new versions or new Consolidated containers with clear provenance.

## 10. Adapters

Non-native stores can connect through light adapters. An adapter must wrap
external results into at least:

- `type="Original"`
- `summary`
- `source_ref`
- `trust`
- `content_mode`
- `available_modes`

Adapters should prefer `AnchoredSummary` or `Excerpt` over `Full` when possible.

## 11. Trust and Safety

P-Mem must preserve PCP trust labels. It should not upgrade trust merely because
a page has been stored for a long time.

Recommended behavior:

- External raw content enters as `audited` only after an Auditor or equivalent
  review.
- Consolidated pages can become `sealed` only when all sources satisfy PCP's
  sealed-trust conditions.
- Durable deletion or durable negative weighting should require explicit
  high-confidence feedback.
- Fetching `Full` content from external sources may require re-audit before
  injection into a Worker context.

## 12. Minimal Implementation Plan

Phase 1: file-backed page store

- JSONL or SQLite Page Record storage.
- `QueryMemory` over keywords, anchors, source refs, and graph metadata.
- `FetchMemory` by `id`, `source_ref`, and `source_spans`.
- `content_mode` / `available_modes` enforcement.

Phase 2: model-assisted Router

- Full Intent Focus ranking.
- `desired_content_mode` prediction.
- query explanations and recall diagnostics.

Phase 3: Consolidator jobs

- duplicate detection;
- topic-root creation;
- horizontal merge into Consolidated Pages;
- stale and superseded page marking.

Phase 4: adapters

- file-system adapter;
- structured-document adapter;
- generic search/RAG adapter.

## 13. Open Questions

- Whether `version` should be a monotonic integer, content hash, or hybrid.
- Whether `trust` should remain a flat enum or gain application-specific
  sublabels.
- How much Router reasoning trace should be persisted for audit.
- Whether high-value page detection should be domain-specific, for example
  dependency-graph centrality in a domain-owned structured corpus.
