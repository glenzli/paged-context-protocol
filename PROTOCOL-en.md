# Paged-Context-Protocol (PCP) - v0.4.0-draft

> **Status: Draft.** This document redefines PCP Core. It is not backward
> compatible with the archived
> [v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md) generation.

Paged-Context-Protocol (PCP) is a model-facing, user-owned logical context
storage protocol. It allows different models to access and maintain context
across sessions, projects, and time through ordinary tool interfaces without
requiring a fixed routing, compaction, zooming, or reasoning workflow.

PCP's core position is:

> **Define a model-addressable backing store for context, not a procedure for
> running the model.**

## 0. Normative Language and Scope

The key words MUST, MUST NOT, SHOULD, and MAY in this document indicate
normative requirements.

PCP Core defines:

- the semantics of Pages, Revisions, Scopes, Provenance, and Relations;
- operations for writing, reading, searching, revising, linking, and managing
  the lifecycle of Pages;
- the recovery boundary between raw history and model-derived content;
- interoperability constraints across models and storage implementations.

PCP Core does not define:

- when a model searches or writes;
- whether a model uses grep, full-text search, semantic retrieval, or graph
  traversal;
- how the active context window is compacted, folded, or ordered;
- a fixed Router, Worker, Consolidator, or Auditor topology;
- fixed prompts, XML templates, Chain-of-Thought, or zoom state machines.

## I. Design Principles

### 1.1 The Model Owns Its Active Context

The current model or its harness manages the active working set. PCP does not
control reading order, reasoning method, summarization strategy, or tool-use
habits inside an effective context window.

### 1.2 The User Owns Long-Term Context

Long-term context should not be bound to one model, session, project directory,
or provider. A conforming PCP Store should allow different authorized models to
access the same logical address space.

### 1.3 Page Identity Is Independent of Residency

Whether a Page is present in a model window, cached, or only in persistent
storage does not change its logical identity. Active context is a temporary
projection of a Page.

### 1.4 Raw History Provides Recoverability; Derived Pages Provide Usability

When user authorization and retention policy allow it, a Host should preserve
searchable raw conversations, tool events, and source records. Models may create
summaries, conclusions, relations, and other derived Pages over them, but derived
content must not destroy its sources.

### 1.5 Constrain Semantics, Not Strategy

The protocol strictly constrains identity, revision, scope, provenance, and
write behavior while allowing models to choose retrieval and context assembly
strategies. As models become more capable, the protocol manual should become
thinner without weakening persistence boundaries.

## II. System Roles and Boundaries

PCP defines logical roles only. It does not prescribe deployment topology.

### 2.1 Model Client

A model or agent that calls PCP interfaces. It may search, read, write, revise,
and link Pages in any order. Different Model Clients may use entirely different
tool habits.

### 2.2 Host

The application connecting conversations, projects, tools, and model runtimes to
a PCP Store. The Host is responsible for:

- authentication, authorization, and Scope selection;
- capturing raw events when permitted;
- enforcing token, latency, and result-count budgets;
- exposing PCP as MCP tools, function tools, a CLI, HTTP, or local functions;
- injecting retrieved results into the model's active context.

### 2.3 PCP Store

The system that persists Pages, Revisions, Relations, raw events, and indexes.
Implementations may use files, SQLite, relational databases, search engines,
object stores, or hybrid backends.

### 2.4 Adapter

A component that maps external files, repositories, messages, databases, or
other memory systems into PCP Pages. An Adapter MUST preserve retrievable source
information and MUST NOT present a model-generated summary as the raw source.

## III. Logical Address Space and Scope

### 3.1 Unified Does Not Mean Globally Injected

PCP uses a unified logical address space, but every Page does not automatically
appear in every task. Unified addressing only means that Pages are accessed
through consistent semantics. Visibility and recall boundaries are determined
by Scope and authorization.

### 3.2 Scope

Every Page Revision MUST declare:

- `owner_id`: the owner of the long-term context;
- `namespace`: its primary namespace, such as user, project, task,
  conversation, or branch;
- `visibility`: an implementation-defined visibility value or ACL reference.

A Page may be linked to another Scope through a Relation, but that link MUST NOT
grant read access by itself.

Recommended namespace forms:

```text
user:<user-id>
project:<project-id>
task:<task-id>
conversation:<conversation-id>
branch:<branch-id>
```

### 3.3 Recall Scope

A Search request MUST explicitly state its allowed Scopes or reference a Scope
Policy resolved by the Host. An implementation MUST NOT silently expand into an
unauthorized project because of semantic similarity.

A Host may offer policies such as the following, but they are not fixed Core
enums:

- `strict`: the current Scope only;
- `linked`: the current Scope plus explicitly linked Scopes;
- `global`: a user-authorized global range.

## IV. Pages and Revisions

### 4.1 Page

A Page is the smallest persistent logical object that a model can independently
address and compose. A Page is not a fixed token chunk and does not require a
summary.

A Page may represent:

- a raw conversation or tool event;
- a reference to a file, code, image, or another source;
- a user idea, question, or constraint;
- a model-generated note, conclusion, or summary;
- a logical composition of other Pages;
- project state, a decision, counterexample, rejected path, or open question.

### 4.2 Stable Identity and Immutable Revisions

- `page_id` identifies a logical object across time.
- `revision_id` identifies one immutable version of that object.
- Revising a Page MUST create a new `revision_id`.
- When a read specifies only `page_id`, the Store SHOULD return the latest
  effective Revision visible to the caller and MUST identify the actual
  `revision_id` returned.
- Write operations SHOULD accept `expected_revision_id` to detect concurrent
  update conflicts.

A `page_id` MUST NOT rely only on a collision-prone short hash. Implementations
may use UUIDs, ULIDs, full content hashes, or another collision-safe identifier.

### 4.3 Minimal Page Envelope

Every Page Revision MUST contain:

```json
{
  "page_id": "pg_01...",
  "revision_id": "rev_01...",
  "owner_id": "user_01...",
  "namespace": "project:formal-math",
  "visibility": "private",
  "lifecycle_status": "active",
  "created_at": "2026-07-15T20:00:00+08:00",
  "created_by": {
    "actor_type": "user|model|tool|system",
    "actor_id": "..."
  },
  "payload": {
    "media_type": "text/markdown",
    "content": "..."
  },
  "source_refs": [],
  "provenance": [
    {
      "operation": "write",
      "actor_type": "user",
      "actor_id": "...",
      "timestamp": "2026-07-15T20:00:00+08:00",
      "input_revision_ids": []
    }
  ]
}
```

At least one of `payload` and `source_refs` MUST be non-empty. A Store may keep a
large payload in external object storage while retaining a stable reference in
the Page. `provenance` MUST contain at least the creation, ingestion, or import
event for this Revision.

### 4.4 Free-Form Payload and Optional Facets

The protocol does not fix a domain schema for payloads. An implementation SHOULD
declare a media type or schema identifier, for example:

```json
{
  "payload": {
    "media_type": "text/markdown",
    "content": "..."
  },
  "facets": {
    "summary": "...",
    "keywords": ["compactness", "finite product"],
    "anchors": ["Theorem 4.7", "Definition 2.1"],
    "symbols": ["X", "K_i"]
  }
}
```

Every member of `facets` is optional. A Page remains conforming when a model does
not generate a summary. Facets are retrieval and reading aids and MUST NOT be
treated as replacements for source evidence.

### 4.5 Source-Backed and Derived Pages

PCP does not require a closed Page-type enum, but implementations MUST be able to
distinguish:

- **Source-backed** Pages, which contain a raw event or a stably retrievable
  source;
- **Derived** Pages, which are inferred, organized, or compressed from other
  Pages by a model, rule, or background task.

A Derived Page MUST point to its input Revisions through `provenance` or
`derived_from` Relations. It MUST NOT silently overwrite a Source-backed Page.

## V. Provenance and Relations

### 5.1 Provenance

Provenance records how content entered the Store. Each event should include at
least:

```json
{
  "operation": "ingest|write|revise|derive|import",
  "actor_type": "user|model|tool|system",
  "actor_id": "...",
  "timestamp": "...",
  "input_revision_ids": [],
  "tool_or_model": "optional identifier"
}
```

Provenance establishes a source chain; it does not prove truth. Factual
reliability, instruction authority, and data integrity MUST NOT be collapsed
into one `trust` enum.

### 5.2 Relation

A Relation is a typed, directed edge with addressable endpoints and authorship:

```json
{
  "relation_id": "rel_01...",
  "from_revision_id": "rev_a",
  "type": "depends_on",
  "to_revision_id": "rev_b",
  "created_by": {"actor_type": "model", "actor_id": "..."},
  "created_at": "..."
}
```

Core recommends, but does not close, the following vocabulary:

- `contains`
- `derived_from`
- `depends_on`
- `defines`
- `uses`
- `supports`
- `contradicts`
- `supersedes`
- `inspired_by`
- `related_to`

Domain Adapters may add Relation types. A Store MUST preserve the original type
and MUST NOT collapse all relationships into untyped similarity edges.

## VI. Raw Events and Model Memory

### 6.1 Raw Event Stream

To avoid losing information when a model forgets to write or judges something
too early, a Continuity Host SHOULD preserve the following as Source-backed
Pages or searchable Events within the authorized retention boundary:

- user and model messages;
- tool calls and necessary results;
- project, task, and branch identifiers;
- file or external-state changes;
- content the user explicitly asks to retain.

A raw event is not active context by default and does not need to be recalled on
every turn.

### 6.2 Model-Maintained Derived Layer

Models may use the ordinary Write, Revise, and Link interfaces to create higher
quality long-term Pages, including:

- current project state;
- important decisions and rationale;
- cross-project ideas;
- verified conclusions;
- failed paths and negative results;
- summaries or indexes over historical events.

Background maintenance models and foreground working models use the same
interfaces. PCP does not define a separate Consolidator processor.

### 6.3 Non-Destructive Organization

Summarization, merging, deduplication, and reorganization MUST produce new
Revisions, Relations, or Derived Pages. They MUST NOT remove the only raw source.
Physical deletion is controlled by user authorization and Host retention policy.

## VII. Core Interfaces

This section defines logical semantics, not transport. Implementations may extend
fields but MUST preserve the core behavior.

### 7.1 DescribeCapabilities

Allows a Model Client to discover Store capabilities. The result SHOULD include:

- supported search modes;
- supported Projections;
- supported Scope types, policies, and discovery capabilities;
- pagination, result-size, and payload limits;
- supported Relations and schema extensions;
- support for event ingestion, revision conflict detection, and durable deletion.

A model should not depend on an undeclared capability.

### 7.2 ListScopes

Lists or searches the Scopes visible to the caller with pagination. This allows
a model to discover historical and project spaces beyond its current project
without already knowing their namespaces. Results SHOULD include:

- the Scope identifier, display name, and optional description;
- Scope type and parent or explicit links;
- caller permissions;
- last activity time and optional Page-count statistics;
- a pagination cursor.

ListScopes exposes only authorized metadata. Listing a Scope MUST NOT read Page
content inside that Scope.

### 7.3 SearchPages

Returns candidate Page Revisions from authorized Scopes.

```json
{
  "query": "Did we previously discuss a proof that finite products preserve compactness?",
  "scopes": ["project:formal-math", "user:user_01"],
  "mode": "auto",
  "filters": {
    "relation_types": ["depends_on", "inspired_by"],
    "created_before": null,
    "lifecycle_status": ["active", "superseded"]
  },
  "limit": 20,
  "cursor": null
}
```

Search modes may include:

- `exact`: exact strings, symbols, or IDs;
- `text`: grep, regex, full-text search, or BM25;
- `semantic`: semantic candidate recall;
- `graph`: Relation traversal;
- `temporal`: time and revision queries;
- `hybrid`: multiple retrieval channels;
- `auto`: selected by the Store or a Model Client Adapter.

PCP does not define the final relevance algorithm. Semantic scores, full-text
scores, and graph distance MUST NOT be presented as a naturally comparable
single ground-truth value.

Search results MUST include at least:

- `page_id` and `revision_id`;
- Scope;
- a matching excerpt or available facets;
- the match channel and compact match metadata;
- available read Projections;
- a pagination cursor.

### 7.4 ReadPages

Reads known Pages. Recommended Projections include:

- `manifest`: Envelope and available capabilities;
- `payload`: content of the current Revision;
- `source_spans`: selected source ranges;
- `relations`: incoming, outgoing, or typed Relations;
- `facets`: optional summaries, anchors, symbols, and indexes;
- `history`: Revision history.

A Projection belongs to a read request, not to persistent Page state. PCP does
not define a `Summary -> Detail -> Unpacked` state machine.

### 7.5 WritePage

Creates a Page. A request SHOULD support:

- payload or source references;
- Scope;
- optional facets;
- optional initial Relations;
- provenance;
- an idempotency key.

The Store MUST return the final `page_id` and `revision_id`.

### 7.6 RevisePage

Creates a Revision for an existing `page_id`. The request SHOULD contain
`expected_revision_id` or explicitly permit a branch from an older Revision. A
Store MUST NOT overwrite a published Revision in place.

### 7.7 LinkPages

Creates a typed Relation between Revisions. An implementation may version
Relations independently, but it MUST at least preserve authorship, time, and both
endpoints.

### 7.8 SuppressPages

Reduces or prevents recall within a specified task, conversation, or query
scope. Suppression does not change durable Page state and MUST NOT automatically
become global negative feedback.

### 7.9 TombstonePage and DeletePage

- `TombstonePage` marks a Page as obsolete, withdrawn, or excluded from ordinary
  recall while preserving audit and graph integrity.
- `DeletePage` physically deletes data and MUST be controlled by user authority,
  retention policy, and applicable regulation.
- A Model Client may recommend a tombstone but should not have default authority
  to irreversibly delete user history.

### 7.10 IngestEvent

A Host-facing interface for raw messages, tool events, and project-state
changes. Ingest MUST support idempotency keys to avoid duplicate history after
retries.

## VIII. Retrieval and Model Autonomy

### 8.1 No Prescribed Search Plan

A model may:

- list Scopes and then run grep;
- search for a symbol and then traverse `depends_on`;
- perform semantic recall and then read sources page by page;
- query a conversation directly by time;
- run multiple retrieval modes in parallel and compare them itself.

None of these behaviors is a protocol state machine.

### 8.2 Structural Constraints and Semantic Discovery Can Coexist

A domain system may treat explicit dependencies as mandatory candidates while a
model discovers relationships not yet represented by edges. A mathematical
system, for example, may compute a theorem dependency closure before asking a
model to add analogies, historical discussions, and hidden premises. PCP does
not mandate which result has final priority, but interfaces MUST preserve result
origin and Relation type.

### 8.3 Context Bundle

A model or Host may compose several Page Projections into one model input. A
Context Bundle may be an ephemeral response or a persisted Derived Page, but it
is not a fixed PCP Core runtime container.

A Bundle SHOULD preserve the `page_id`, `revision_id`, and source of every
fragment. It should not concatenate Pages into untraceable text.

## IX. Lifecycle and Time

### 9.1 Multiple Time Dimensions

Implementations SHOULD distinguish:

- `created_at`: when the Revision was written to the Store;
- `observed_at`: when the source event happened or was observed;
- `valid_from` / `valid_to`: when the content applies to the world or project
  state, when relevant.

Storage time alone MUST NOT determine factual freshness.

### 9.2 Updates and Conflicts

New information may:

- revise the same Page;
- `supersedes` an older Revision;
- `contradicts` another Page;
- remain simultaneously valid under different Scopes or conditions.

A Store SHOULD preserve these distinctions and MUST NOT automatically deduplicate
conflicting content merely because the text is similar.

### 9.3 Lifecycle Status

Core defines the following `lifecycle_status` values:

- `active`: participates in ordinary recall;
- `superseded`: replaced by newer content but explicitly readable;
- `archived`: excluded from ordinary search by default but explicitly readable;
- `tombstoned`: withdrawn or obsolete and visible only under audit or recovery
  policy.

A lifecycle transition MUST create a new Revision or an independent, auditable
lifecycle event. It MUST NOT mutate an immutable Revision in place. Archive,
Suppress, Tombstone, and Delete are distinct operations.

## X. Security, Permissions, and Epistemic Metadata

### 10.1 Authorization Before Relevance

The Store MUST enforce access control before retrieval and reading. An
unauthorized Page MUST NOT enter candidate results even when highly relevant.

### 10.2 Cross-Scope Recall Must Be Explicit

Cross-project or user-level recall must be allowed by request scope or an
explicit Relation. Results MUST carry their original Scope so that the model and
user can identify cross-project content.

### 10.3 Data Does Not Automatically Become Instruction

External sources, historical conversations, and model-derived Pages SHOULD be
treated as data by default. The Host defines which sources may provide runtime
instructions and limits worst-case impact through tool permissions and execution
boundaries.

### 10.4 Do Not Replace Multiple Questions with One Trust Axis

An implementation may separately record:

- `authority`: who may issue instructions or mutate state;
- `integrity`: whether content is raw, derived, signed, or verified;
- `epistemic_status`: asserted, corroborated, contradicted, superseded, and so on;
- `sensitivity`: data sensitivity;
- `instruction_policy`: data-only or eligible as an instruction source.

These fields describe different dimensions. Passing an audit MUST NOT imply that
content is factually true.

### 10.5 User Control

Users MUST be able to inspect, export, restrict, and delete their long-term
context. Hidden model-generated summaries MUST NOT become the only unauditable
copy of memory.

## XI. Transport and Access Surfaces

PCP is a semantic protocol, not a transport protocol. Compatible
implementations may expose:

- MCP tools;
- JSON-RPC or HTTP APIs;
- local function libraries;
- a CLI;
- a file-system view;
- SQLite or another query interface.

One Store may expose several access surfaces at once. A shell-oriented model may
use grep and a CLI, while a structured-tool model uses a JSON API. Both MUST
resolve to the same Page and Revision identities.

## XII. Minimal Conformance Requirements

A **PCP Core Store** MUST at minimum:

1. persist stable `page_id` values and immutable `revision_id` values;
2. enforce Scope and access control;
3. support the logical semantics of Describe, ListScopes, Search, Read, Write,
   and Revise;
4. preserve provenance and source references;
5. distinguish persistent Pages from per-read Projections;
6. avoid replacing the only raw source with a summary;
7. support pagination or result budgets so that no request returns unbounded
   context.

A **PCP Continuity Host** SHOULD additionally:

1. ingest conversation and project events within the authorized boundary;
2. pass the current model identity, Scope, and permissions to the Store;
3. expose capability discovery and basic Page operations to models;
4. preserve Page and Revision references in retrieval results;
5. reserve irreversible deletion for users or explicit retention policy.

A **PCP Adapter** MUST at minimum:

1. provide a stable `source_ref` for external content;
2. identify import and derivation steps;
3. avoid using a temporary external ranking score as Page identity;
4. retain source-location information that the caller is authorized to preserve.

## XIII. Non-Goals

PCP does not attempt to:

- replace provider context windows or native compaction;
- guarantee that a model calls memory at the correct time;
- prescribe one retrieval, indexing, or ranking algorithm;
- automatically inject all history into every request;
- determine truth from storage age or model summaries;
- replace application-level permission, privacy, or execution-security systems.

## XIV. Open Questions

- Should Page payload schemas remain fully open, or should PCP define a small set
  of standard profiles?
- Do Relations need independent revision and permission models?
- Should Context Bundles standardize budget and ordering metadata?
- How should multiple models express disagreement over the same Derived Page?
- How should user-controlled deletion preserve graph consistency?
- Which events should a Continuity Host ingest by default, and which require
  explicit authorization?
- How should cross-model conformance be evaluated on millions of tokens of
  high-density context?
