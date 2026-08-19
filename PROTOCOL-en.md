# Paged-Context-Protocol (PCP) - v0.8.0-draft

> Status: Draft. v0.8 defines Identity, tenant, and Runtime maintenance authority above the Page/Revision model and narrows source and relation semantics.

Paged-Context-Protocol (PCP) is a model-facing, user-owned protocol for durable context. Authorized Hosts and models can discover, read, write, and trace one persistent information space without adopting a fixed retrieval, summarization, compaction, or reasoning workflow.

PCP's boundary is:

> **Accept multi-tenant input into a user-owned Identity, maintain stable Pages, immutable Revisions, sources, and cross-Scope structure, and provide authorized models with the most relevant traceable working context.**

## 1. Core objects

### Identity, Principal, and Scope

An Identity is PCP's durable context and maintenance boundary. Any number of tenants may contribute to one Identity. Runtime may discover and maintain cross-tenant structure inside it, but MUST NOT infer, connect, or recall across Identities. The official Runtime currently binds one Store to one Identity.

Tenants act through server-injected Principals and AccessSessions. A Principal is a caller, not the owner of the information space. A Scope is an authorization slice inside an Identity. Unified maintenance does not grant every tenant global visibility. If either endpoint of a Relation is unreadable, a response MUST NOT reveal the Relation or the hidden endpoint.

Clients read `identityId` from the Runtime descriptor rather than asserting ownership themselves. Information requiring an independent maintenance and association boundary belongs to a separate Identity, not a tenant-named pseudo-boundary.

### Page

A Page is the smallest semantic segment worth recalling independently. It is neither a source-system event nor a content version. It has a `pageId`, `headRevisionId`, Scope namespace, open `kind`, `mutability`, lifecycle, and timestamps.

- `sealed` Pages represent raw messages, file snapshots, tool results, and other evidence. They cannot publish a second Revision. Unreferenced leaves may be atomically replaced by an equivalent packed Page under the lossless rules below.
- `revisioned` Pages represent maintained understanding such as Summaries, Topics, profiles, and project state.
- Mutability is a content invariant, not a retention tier.
- Lifecycle controls default discovery and does not assert truth.

### Revision

A Revision is an immutable Page snapshot with `revisionId`, `pageId`, optional `previousRevisionId`, payload or source references, creator, times, facets, and provenance. A revisioned Page advances its head through compare-and-swap. Ordinary version history uses `previousRevisionId`; it MUST NOT create an intra-Page `supersedes` Relation. A collected Revision MUST fail exact reads explicitly rather than silently resolving to the current Page head.

Immutable does not mean retained forever. A Store may reclaim unprotected historical Revisions. Current heads, sealed evidence, provenance inputs reachable from protected roots, Relation basis Revisions, and explicit retention roots remain protected. Default Search and full-text indexes cover Page heads only.

`createdAt` records Store commit time while optional `observedAt` records source-event time. They are not interchangeable. An optional `sourceSpan` is a closed range `{streamId, start, end}` in one producer-local event stream. Runtime namespaces an ordinary ingest stream by authenticated Principal. A SourceSpan proves order and coverage; it is not a Page Relation or a semantic-similarity assertion.

### SourceRef and external media

A Page is the stable PCP identity of its source. When another system retains the original, a Revision may point to it with one minimal `SourceRef`:

```json
{
  "providerId": "tenant:photos",
  "locator": "opaque-photo-42",
  "mediaType": "image/jpeg",
  "contentDigest": "sha256:..."
}
```

`providerId + locator` has meaning only to the custodian; it is not permission for Runtime to fetch an arbitrary path or URL. `mediaType` and `contentDigest` are optional. The digest verifies returned content without creating a second asset identity. SourceRef does not promise availability; resolution failure leaves the Page and existing semantic representations intact.

Searchable OCR, transcript, caption, layout, event, or domain interpretation is stored as an ordinary Page/Revision with exact provenance to the media-bearing Revision. One image may have several task-specific representations. Byte custody, provider callbacks, and automatic extraction are outside the v0.8 Core.

### Relation

A Relation is a maintainable semantic assertion between stable Pages. Optional `basisRevisionIds` capture the exact versions observed when the assertion was made. Navigation follows Pages; audit follows Revisions. `relationType` is an open string and Capabilities do not enumerate a vocabulary; the following types have Core-defined semantics.

Core conventions include `summarizes`, `assesses`, cross-Page `supersedes`, `aggregates`, `about`, and `related_to`. Pages about the same subject SHOULD point to one stable Topic Page with `about`, rather than form a pairwise clique. `related_to` is the symmetric fallback for a direct conceptual association worth future joint recall when no more precise type applies. Runtime canonicalizes its endpoints by Page ID and stores one logical edge. It does not express sequence, provenance, replacement, or dependency.

Exact generation dependency belongs in Revision provenance; a `derived_from` Page Relation is added only when it also has navigation value.

Relations MUST NOT be inferred from temporal adjacency, shared Scope, co-retrieval, or vector similarity. Conversation order belongs to a Host event stream. Duplicate active triples are coalesced. Unconfirmed candidates remain in the maintenance ledger; they are not Relations. Incorrect Relations may be retracted without mutating either Page.

### Provenance

Provenance belongs to a Revision and references the exact input Revision IDs actually used. It records operation, actor, time, tool or model where useful, and external sources. Relations support navigation; provenance reconstructs what a generation depended on.

## 2. Summary, validity, and lossless packing

A Summary is an ordinary revisioned Page connected to its target by `summarizes`. Only long, dense, or future-useful content needs one. A better Summary publishes a new Revision of the same Summary Page instead of creating another Page.

When a long-lived topic spans several Pages, a Runtime maintainer may execute
`extract_topic(sourcePages[{pageId, revisionId}], title, content)`. It creates a separate revisioned
Topic Page with `kind = topic_summary`, retains exact provenance for every input, and writes a
`summarizes` Relation from the Topic to every source Page. Inputs MUST be 2-64 distinct, active,
current Revisions in one Scope; a Topic cannot be a Topic input. This is **logical extraction**:
source Pages and exact Revisions are not deleted, remain readable by ID, and may return as evidence
through high-relevance Relation expansion. Only the default candidate surface for `semantic_search`
and `match_intent` is represented by the current Topic Page. A Topic update publishes a new Revision;
it suppresses a source from default candidates only while that current Revision still lists the same
source Revision.

Typical recall is:

```text
Search/Browse current Summary and Page heads
  -> model selects stable Page IDs
  -> Read current or exact Revisions
  -> optionally follow Page Relations or exact provenance
```

Validity assessments are ordinary Pages as well. Page lifecycle controls discovery; standings such as live, qualified, disputed, or retracted come from assessment content and evidence.

Lossless packing reduces the object count of fine-grained sealed Pages without asking a model to rewrite or omit source content. Runtime may ask a semantic worker to choose an ordered subset from a bounded candidate window, but Store performs the commit deterministically.

`pack_pages` accepts 2-64 unique exact current Revisions in the same Scope and `kind`, ordered as a strictly contiguous range in one `sourceSpan.streamId`. Ordinary inputs MUST be active, sealed, single-Revision leaves with no Page Relation, Relation basis or retraction, cross-Revision provenance dependency, Summary, Validity record, or retention lease. At most one input may instead be an active, revisioned packed Page acting as a stable anchor. Existing Relations, Summaries, Validity records, and historical Revisions on that anchor do not block extension because its Page identity is retained.

Without an anchor, Store creates a revisioned Page. With an anchor, Store CAS-publishes a new Revision on that same Page. The `entries` of `application/vnd.pcp.packed-page+json` always contain the original leaves in source order and MUST NOT contain another packed payload; extension flattens rather than nests. In one transaction, Store removes only newly absorbed sealed leaves and records them in a content-free pack ledger. Exact reads of retired leaf IDs report the stable packed Page explicitly and never redirect silently. The previous anchor Revision remains ordinary history.

v0.8 does not merge two existing packed Pages or pack across a `sourceSpan` gap. Such content remains separate and may be organized with `related_to`, `about`, Topics, or another Relation. Runtime may use temporal proximity and semantic continuity to select candidates, but model judgment cannot weaken Store invariants.

Physically deleting original detail after a Summary, Topic, or other representation matures is lossy
condensation. It requires separate quality, recovery, confirmation, and audit semantics and is
deferred beyond v0.8. `extract_topic` changes only default routing, and `pack_pages` MUST NOT
implement physical condensation.

## 3. Interface semantics

### 3.1 Tenant data plane

The normative ordinary-tenant surface is deliberately small: `describe() -> identityId, access, capabilities`, `list_scopes(query?, limit?, cursor?)`, `ingest_page(namespace, kind, payload?, source_refs?, observed_at?, source_span?, facets?, external_event_id?)`, bounded `search_pages(query, scopes, strategy?, limit?, cursor?)`, current or authorized exact `read_pages(page_ids, revision_ids?, view?, max_chars?)`, `semantic_search(query, scopes?, result_limit?, context_budget_chars?)`, `match_intent(query, scopes?, result_limit?, context_budget_chars?, intent_effort?)`, and anchored `expand_graph(anchor_page_ids, scopes?, max_depth?, max_nodes?, max_edges?, view?, max_chars?)`. Implementations may additionally expose bounded `browse_index(scopes?, view?, limit?, cursor?)` as a capability.

`ingest_page` is the tenant's only durable write. Identity comes from the connected Runtime and is not repeated on every Page, Revision, Scope, or request. Runtime fills Actor, active lifecycle, and sealed mutability from the authenticated session and isolates an optional `sourceSpan.streamId`. A `read` session retrieves only; a `contribute` session additionally receives a distinct `ingest` permission and does not thereby gain advanced Page or maintenance writes.

Search returns bounded candidates rather than truth, is paginated, and identifies the matched projection and current Revision. Authorized Relation, Summary, Validity, provenance, and SourceRef projections are read through the same tenant surface rather than separate mutation APIs. A caller holding an exact Revision ID may read that historical evidence when its Scope grants allow it, but historical Revisions do not re-enter default Search; raw access audit and history enumeration remain audit or operator concerns. The consuming model chooses queries and assembles the active working context.

`search_pages` is a deterministic candidate and debugging interface, not a model's default intelligent retrieval tool. `semantic_search` is the public Runtime-owned conservative semantic endpoint: it returns independently relevant Pages and uses asserted Relations only as bounded rank adjustments. `match_intent` lets a Router expand intent and review candidate and relation leads within a `low`, `medium`, or `high` budget. Both return structured entries with `pageId`, `revisionId`, inclusion reason, audit, and projected content; they do **not** return a fixed prompt wrapper. The caller decides how to place entries into a model context. Empty `scopes` means all Scopes already authorized for the session, and every final Page read remains Store ACL checked. Each endpoint without its required provider MUST directly report the unavailable method and recovery condition rather than fall back to keyword search.

`expand_graph` starts from explicit `anchor_page_ids` and is bounded by depth (implementation maximum 3), node count, and edge count. It returns only Relation/provenance edges and nodes authorized at every hop. PCP provides no unanchored whole-graph export and does not misrepresent Console-only source-stream virtual edges as protocol Relations. `pageId` is the stable object identity and graph anchor; `revisionId` identifies exact reviewable historical evidence.

### 3.2 Runtime maintenance surface

Runtime maintainers and local administration tools may use the complete Core operations: advanced sealed or revisioned Page writes, CAS revision, atomic `pack_pages(pages[{page_id, revision_id}], idempotency_key?)`, logical `extract_topic(source_pages[{page_id, revision_id}], title, content)`, Summary and validity writes, Page Relations, Scope management, audit, bounded retention planning, explicit collection, and finite idempotent Revision retention leases. These implement Identity-wide maintenance policy and are not part of the ordinary tenant contract.

An implementation may carry both operation sets over one RPC transport, but it MUST enforce the boundary through session permissions and an operation allowlist. The interface split does not require another socket or deployment unit.

### 3.3 Control and observation planes

Runtime Discovery, enrollment, approval, and `open_session` belong to Runtime control protocols. Health, Observer snapshots, raw audit, and maintenance controls belong to read-only observation or local operator interfaces. They may ship with PCP, but are not the tenant Page data plane and are not carried as Core Page requests.

## 4. Responsibility boundaries

The protocol defines the Identity boundary, Page and Revision identity, sealed/revisioned invariants, Page Relations, exact provenance, SourceRef, Scope authorization, CAS publication, and retention safety constraints.

Store and Runtime own transactions, current-head indexes, authorization enforcement, relation retraction, retention, GC roots, candidate discovery, and Identity-wide Summary, Validity, lossless-packing, semantic-relation, and retention policy. Runtime may invoke a separate model as an inference provider, but owns task generation, budgets, validation, commit authority, and the maintenance ledger.

A tenant or Host captures its own source events, source-local ordering and deterministic structure including optional SourceSpans, Page kind, SourceRefs, and external-media custody. It may submit feedback or candidates but does not own the global relation graph. The consuming model chooses queries, reads authorized exact content, judges task relevance, and assembles the active working context.

PCP does not define a fixed prompt, vector algorithm, summarization threshold, background-agent topology, or user-profile schema.

## 5. Retention

Implementations may classify Revisions as current, protected, reclaimable, cold, or stubbed, but those are Store states rather than protocol fields. Current heads, sealed evidence, provenance inputs reachable from protected roots, Relation-basis inputs, and explicit snapshots or leases are protected. Lossless packing under Section 2 is the only v0.8 exception for sealed leaves. Unreferenced intermediate Revisions may be compacted or deleted after roots are recalculated. IDs are never silently reused.

An exact read of a collected Revision MUST report that it is unavailable and MUST NOT fall back to the current Page head. `previousRevisionId` records publication order rather than permanent retention, so a retained history may contain physical gaps.

Before collection, a Runtime should expose a deterministic dry-run plan. The plan reports scanned and protected counts, candidate Revision and Page counts, protection reasons, estimated candidate bytes, and bounded candidate and protected samples. Estimated bytes compare candidate payload size; they do not promise immediate database-file shrinkage.

The planner begins from current heads, sealed evidence, recent-version and minimum-age windows, Relation basis Revisions, current projections, live idempotency records, explicit snapshots, and leases. Protection then closes over cross-Page provenance reachable from protected roots. Provenance owned only by a candidate does not keep the whole candidate subtree alive, and ordinary same-Page `previousRevisionId` links are not GC roots. A dependency crossing an unauthorized Scope conservatively protects the authorized input without exposing the outside object.

Explicit retention uses finite, idempotently renewable Revision leases rather than Page content fields. A lease binds an exact Revision, authorized Scope, holder Principal, reason, and expiration; an expired lease is no longer a protection root. A Runtime may offer bounded routing views of actual collection candidates to a Host semantic worker, but the model selects candidates and reasons only. It does not choose global GC policy or bypass Store authorization and protection closure. Permanent retention, early revocation, and collection remain explicit operator actions rather than silent consequences of an ordinary model decision.

A dry run performs no deletion. Collection requires a separately authorized caller to submit exact candidate Revision IDs. The Store MUST recalculate roots inside the write transaction and reject the entire batch if any ID is no longer eligible. A successful collection atomically removes candidate Revisions, candidate-owned projection indexes or source edges, and only past-window idempotency records linked to those candidates. It keeps replay metadata for current heads and surviving operations, and writes a content-free collection ledger so an old Revision ID remains distinguishable from one that never existed. Implementations list only optional capabilities in `capabilities.features`; retention planning and application use `revision_retention_planning` and `revision_retention`, respectively. Planning support does not imply collection support. Mandatory v0.8 behavior is not repeated as always-true booleans.

Retention policy is configured by Identity, Page kind, storage budget, and value; tenants and models should not choose GC parameters on every write.

## 6. v0.8 version boundary

v0.8 is not wire- or Store-schema-compatible with v0.7. Upgrade creates a new v0.8 Store and imports original tenant-held content through `ingest_page`. Implementations MUST NOT open the old database directly or infer Identity, SourceRefs, or new semantic Relations from old URIs, summaries, vector hits, or historical provenance. Advanced writes remain available to Runtime maintainers and administration tools.
