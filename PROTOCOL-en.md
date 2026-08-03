# Paged-Context-Protocol (PCP) - v0.6.0-draft

> Status: Draft. v0.6 removes the earlier two-level Page/Revision object and
> the independent version systems for Summary and Validity. Core now centers
> on immutable Pages, Relations between Pages, and optional Refs.

Paged-Context-Protocol (PCP) is a model-facing, user-owned protocol for durable
context. It lets authorized Hosts and models discover, read, write, and trace a
shared information space without prescribing one retrieval, summarization, or
reasoning workflow.

PCP's boundary is:

> **Maintain an addressable, immutable, traceable Page graph. Leave when to
> recall, how to interpret, whether to write, and what enters current attention
> to the model and Host.**

## 1. Core Objects

### 1.1 Page

A Page is PCP's only persistent content object. A Page:

- has a globally stable `page_id`;
- belongs to one `namespace` (Scope);
- cannot be edited in place after creation;
- may contain a payload, stable references to large external objects, or both;
- may refer to other Pages through Relations;
- may represent either a raw event or a model-derived summary, judgment, or
  aggregate.

Minimal Page envelope:

```json
{
  "pageId": "pg_01...",
  "ownerId": "usr_01...",
  "namespace": "project:example",
  "visibility": "private",
  "lifecycleStatus": "active",
  "createdAt": "2026-08-03T10:00:00+08:00",
  "createdBy": {
    "actorType": "user|model|tool|system",
    "actorId": "..."
  },
  "payload": {
    "mediaType": "text/markdown",
    "content": "..."
  },
  "sourceRefs": [],
  "provenance": []
}
```

At least one of `payload` and `sourceRefs` SHOULD be non-empty. Object storage
may hold large images, audio, and other assets while the Page preserves a
stable reference, media type, routing description, and source.

Implementation-defined `facets` MAY assist indexing. They are not Core
identity, do not replace payload, sources, or Relations, and SHOULD NOT enter
model context unconditionally.

### 1.2 Relation

A Relation is a directed fact between two immutable Pages:

```json
{
  "fromPageId": "pg_new",
  "relationType": "supersedes",
  "toPageId": "pg_old",
  "createdAt": "...",
  "createdBy": { "actorType": "model", "actorId": "..." }
}
```

Core Relations are:

- `supersedes`: the source is a newer interpretation, state, or content
  successor; the target remains readable;
- `summarizes`: the source is a routing Summary for the target;
- `derived_from`: the source directly used information from the target;
- `assesses`: the source judges how the target should currently be used.

Hosts MAY add domain Relations such as `responds_to`, `supports`,
`contradicts`, and `aggregates`. The `summarizes`, `derived_from`, and
`aggregates` derivation subgraph MUST be acyclic. `supersedes` MUST form
traceable directed successor chains without cycles.

A Relation MUST come from an explicit assertion. The Store MUST NOT create a
Relation merely because two Pages are adjacent in time or write order, share a
Scope, co-occur in search results, or are similar in an embedding space. In
particular:

- `responds_to` means a demonstrable reply or generation dependency, not only
  that a user message occurred after the latest assistant message;
- `continues` means semantic continuation and requires judgment by the Host,
  user, or model;
- ordinary conversation order belongs in a temporal projection or Host event
  stream, not in a `follows` edge that pollutes the Page graph.

The Store SHOULD create only structural Relations determined by the current
operation, such as `supersedes`, `summarizes`, and `assesses`, plus
`derived_from` when the caller supplies exact input Pages. Domain-semantic
Relations belong to the Host, user, or model.

A Relation is a maintainable assertion separate from Page content. Retracting
an incorrect, stale, or mechanically generated Relation does not mutate either
endpoint Page. Implementations SHOULD audit the retraction actor, time, and
reason. Exact audit views MAY retain retracted Relations, but default Search
and graph traversal MUST ignore them.

A Relation does not grant access to its other endpoint.

### 1.3 Ref

A Ref is an optional mutable locator:

```text
ref_id -> head_page_id
```

Refs provide stable entry points for concepts such as “current user
orientation” or “current project state.” Advancing a Ref never edits an old
Page. The Host creates a new Page, links it with `supersedes`, and atomically
advances the Ref.

A Ref is not content or evidence and does not participate in the derivation
DAG. Durable provenance MUST resolve to exact Page IDs, not only to a Ref that
may move later.

### 1.4 Scope

Every Page MUST belong to one `namespace`. Recommended forms include:

```text
user:<user-id>
project:<project-id>
task:<task-id>
conversation:<conversation-id>
```

A unified address space is not global injection. Search, Read, graph traversal,
and writes MUST respect the AccessSession's Scope Grants. Semantic similarity
must never silently widen an authorization boundary.

## 2. Summary and Detail

A Summary is not a versioned field on another Page and not a separate storage
object. It is an ordinary derived Page connected to its target by `summarizes`.

Only long, dense, or future-useful content SHOULD receive a Summary. Short or
low-value events may remain available through exact, lexical, or temporal
search without a Summary.

A typical recall path is:

```text
Search/Browse compact routing text
  -> model selects candidate Page IDs
  -> Read exact Page content
  -> optionally follow Relations or provenance
```

A model may instead use exact search, full-text search, graph traversal, or a
known Page ID. PCP does not prescribe a fixed summary-detail state machine.

A better Summary MUST be a new Summary Page that `supersedes` the old Summary.
Cross-Page topic organization is likewise represented by ordinary aggregate or
Summary Pages linked to their exact sources.

### 2.1 Multi-Page Consolidation

A long-running Memory layer cannot only add Summaries and Relations. When a
model determines that several current Pages express one durable subject, fact,
or state, it may write one self-contained Page that `supersedes` every replaced
Page. This operation is consolidation, not summarization: the new Page is
directly usable content for future recall, not merely an index into old Detail.

Consolidation MUST be atomic and:

- take at least two exact Page IDs that are still current;
- select one canonical Page as the result identity; atomically converge every
  input Ref on the new Page and return the canonical Ref;
- record every input and the `consolidate` operation in provenance;
- keep every input exactly readable through lineage while removing it from
  default Search, index browsing, and current graph views;
- refuse to collapse incompatible, contradictory, or merely adjacent Pages,
  which should instead remain separate, aggregate, or receive assessments.

Similarity, temporal adjacency, and shared Scope may discover candidates but
MUST NOT trigger consolidation by themselves. A Host, user, or model owns the
semantic judgment and lossy synthesis. An implementation MAY provide an
optional background maintainer, but PCP does not prescribe its schedule,
similarity threshold, or model.

## 3. Validity and Change

Later information never rewrites Page content. It may:

- create a new Page that `supersedes` an old Page;
- create an assessment Page that `assesses` a target and is `derived_from`
  evidence Pages;
- add `supports`, `contradicts`, or implementation-defined semantic Relations.

When a target has multiple assessments, a newer assessment SHOULD `supersede`
the prior assessment. A Store may project a current standing such as `live`,
`qualified`, `disputed`, `superseded`, or `retracted`, but that projection MUST
trace back to the assessment and evidence Pages that produced it.

Default discovery may return only effective Pages, but exact Page reads and
complete lineage retrieval MUST remain available.

## 4. Provenance

A model- or tool-generated Page SHOULD record:

- creation Actor and time;
- exact input Page IDs;
- the producing operation and tool or model identity;
- required external source references.

The Host SHOULD fill deterministic identity, time, Scope, structural Relations,
and provenance rather than asking the model to reproduce mechanical metadata.
The model supplies content, intent, and the exact Pages it actually used.

Summaries, aggregates, and assessments MUST NOT masquerade as raw evidence.
Derived paths SHOULD remain traceable to sources subject to authorization and
retention policy.

## 5. Model Interface

A Core capability surface SHOULD provide at least:

- `search_pages(query, scopes, strategy?, limit?, cursor?)`
- `read_pages(page_ids, view?, max_chars?)`
- `write_page(content, scope?, based_on_page_ids?)`
- `supersede_page(target_page_id, content, based_on_page_ids?)`
- `consolidate_pages(canonical_page_id, replaced_page_ids, content)`
- `write_summary(target_page_id, content, based_on_page_ids?)`
- `assess_validity(target_page_id, standing, rationale, evidence_page_ids)`
- `relate_pages(from_page_id, relation_type, to_page_id)`

Hosts MAY expose Scope discovery, Ref resolution, audit, and administrative
operations. Default model tools SHOULD remain compact. The Host should produce
structural Relations, Actor, time, idempotency, and routine provenance.

Search results are candidates, not truth. An implementation may use lexical,
exact, temporal, graph, vector, or hybrid retrieval, but it MUST:

- return bounded results and cursors;
- identify the matched Page ID, Scope, projection, and truncated routing text;
- remain inside the AccessSession;
- allow exact Page reads afterward;
- avoid presenting Pages with an incoming `supersedes` Relation as current by
  default while retaining exact access to them.

## 6. Host and Store Responsibilities

The Host owns authentication, AccessSessions, Scope selection, budgets, raw
event capture, event-stream order, domain-semantic Relation judgment, tool
orchestration, and admission into active model context.

The Store owns immutable Pages, caller-asserted Relations, structural
Relations, atomic Ref advancement, atomic consolidation, effective-Page
projection, indexes, authorization enforcement, persistence, audit, and
integrity checks. It does not invent domain-semantic Relations from temporal
adjacency or similarity or decide which content should be lossily consolidated.

PCP does not define user-profile policy, autonomous exploration, interruption
value, fixed prompts, model routing, context-window compaction, or background
agent topology. Those belong to concrete Hosts such as symbiont-d.

## 7. Compatibility and Migration

To migrate a v0.4/v0.5 Page/Revision implementation to v0.6:

1. each old `revision_id` becomes an immutable `page_id`;
2. each old `page_id` becomes an optional Ref;
3. adjacent Revisions of one old logical object become a `supersedes` chain;
4. Summary sidecars become Summary Pages with `summarizes` and required
   `supersedes` Relations;
5. Validity assessments become assessment Pages with `assesses`,
   `derived_from`, and required `supersedes` Relations;
6. migration MUST be idempotent and preceded by a recoverable backup.

A reference implementation may temporarily retain legacy table and Rust type
names internally. Public JSON, model tools, and operator consoles SHOULD expose
the v0.6 Page, Relation, and Ref semantics.
