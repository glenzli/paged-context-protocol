# Paged-Context-Protocol (PCP) - v0.7.0-draft

> Status: Draft. v0.7 restores stable Pages and immutable Revisions while separating semantic identity, version history, and physical retention.

Paged-Context-Protocol (PCP) is a model-facing, user-owned protocol for durable context. Authorized Hosts and models can discover, read, write, and trace one persistent information space without adopting a fixed retrieval, summarization, compaction, or reasoning workflow.

PCP's boundary is:

> **Maintain stable Pages, immutable Revisions, Page Relations, and exact provenance. Leave recall, interpretation, write admission, and active attention to the model and Host.**

## 1. Core objects

### Page

A Page is a stable semantic object, not a content version. It has a `pageId`, `headRevisionId`, owner, Scope namespace, open `kind`, `mutability`, lifecycle, and timestamps.

- `sealed` Pages represent raw messages, file snapshots, tool results, and other evidence. They cannot publish a second Revision.
- `revisioned` Pages represent maintained understanding such as Summaries, Topics, profiles, and project state.
- Mutability is a content invariant, not a retention tier.
- Lifecycle controls default discovery and does not assert truth.

### Revision

A Revision is an immutable Page snapshot with `revisionId`, `pageId`, optional `previousRevisionId`, payload or source references, creator, times, facets, and provenance. A revisioned Page advances its head through compare-and-swap. Ordinary version history uses `previousRevisionId`; it MUST NOT create an intra-Page `supersedes` Relation. A collected Revision MUST fail exact reads explicitly rather than silently resolving to the current Page head.

Immutable does not mean retained forever. A Store may reclaim unprotected historical Revisions. Current heads, sealed evidence, provenance inputs reachable from protected roots, Relation basis Revisions, and explicit retention roots remain protected. Default Search and full-text indexes cover Page heads only.

### Relation

A Relation is a maintainable semantic assertion between stable Pages. Optional `basisRevisionIds` capture the exact versions observed when the assertion was made. Navigation follows Pages; audit follows Revisions.

Core conventions include `summarizes`, `assesses`, cross-Page `supersedes`, and `aggregates`. Exact generation dependency belongs in Revision provenance; a `derived_from` Page Relation is added only when it also has navigation value.

Relations MUST NOT be inferred from temporal adjacency, shared Scope, co-retrieval, or vector similarity. Conversation order belongs to a Host event stream. Duplicate triples should be coalesced, and incorrect Relations may be retracted without mutating either Page.

### Provenance

Provenance belongs to a Revision and references the exact input Revision IDs actually used. It records operation, actor, time, tool or model where useful, and external sources. Relations support navigation; provenance reconstructs what a generation depended on.

### Scope and Alias

Every Page belongs to one namespace. Search, Read, graph traversal, and writes obey AccessSession Scope Grants; similarity never widens authorization.

An Alias is an optional human entry point or compatibility redirect from a name to a Page ID. It is not Page identity, evidence, or a derivation node. Legacy Refs may migrate to Aliases.

## 2. Summary, validity, and consolidation

A Summary is an ordinary revisioned Page connected to its target by `summarizes`. Only long, dense, or future-useful content needs one. A better Summary publishes a new Revision of the same Summary Page instead of creating another Page.

Typical recall is:

```text
Search/Browse current Summary and Page heads
  -> model selects stable Page IDs
  -> Read current or exact Revisions
  -> optionally follow Page Relations or exact provenance
```

Validity assessments are ordinary Pages as well. Page lifecycle controls discovery; standings such as live, qualified, disputed, or retracted come from assessment content and evidence.

Consolidation lossily converges Pages that genuinely represent one durable object. It publishes a new Revision on one canonical revisioned Page, records all exact input Revisions in provenance, links the canonical Page to absorbed Pages with cross-Page `supersedes`, and removes absorbed Pages from default recall. Similarity may discover candidates but never decides the merge.

## 3. Interface semantics

A Core surface should provide bounded Search, current or exact Read, sealed or revisioned writes, CAS revision, atomic consolidation, Summary and validity writes, Page Relations, Scope discovery, audit, a bounded `plan_revision_retention(scopes, policy)` dry run, and finite idempotent Revision retention leases.

Hosts should fill mechanical actor, time, Scope, structural Relation, and provenance fields. Search returns candidates rather than truth and defaults to current heads. Historical Revisions remain exactly addressable for audit but do not re-enter default retrieval.

## 4. Responsibility boundaries

The protocol defines Page and Revision identity, sealed/revisioned invariants, Page Relations, exact provenance, Scope authorization, CAS publication, and retention safety constraints.

The Store and Runtime own transactions, current-head indexes, authorization enforcement, relation retraction, retention, cold storage, GC roots, candidate discovery, and maintenance job lifecycle. Physical residency does not enter model context.

The Host owns event capture and order, Page kinds, revision policy, Summary/Topic/profile policy, semantic judgment, model routing, proactive exploration, attention boundaries, and active-context assembly.

PCP does not define a fixed prompt, vector algorithm, summarization threshold, background-agent topology, or user-profile schema.

## 5. Retention

Implementations may classify Revisions as current, protected, reclaimable, cold, or stubbed, but those are Store states rather than protocol fields. Current heads, sealed evidence, provenance inputs reachable from protected roots, Relation-basis inputs, and explicit snapshots or leases are protected. Unreferenced intermediate Revisions may be compacted or deleted after roots are recalculated. IDs are never silently reused.

An exact read of a collected Revision MUST report that it is unavailable and MUST NOT fall back to the current Page head. `previousRevisionId` records publication order rather than permanent retention, so a retained history may contain physical gaps.

Before collection, a Runtime should expose a deterministic dry-run plan. The plan reports scanned and protected counts, candidate Revision and Page counts, protection reasons, estimated candidate bytes, and bounded candidate and protected samples. Estimated bytes compare candidate payload size; they do not promise immediate database-file shrinkage.

The planner begins from current heads, sealed evidence, recent-version and minimum-age windows, Relation basis Revisions, current projections, live idempotency records, explicit snapshots, and leases. Protection then closes over cross-Page provenance reachable from protected roots. Provenance owned only by a candidate does not keep the whole candidate subtree alive, and ordinary same-Page `previousRevisionId` links are not GC roots. A dependency crossing an unauthorized Scope conservatively protects the authorized input without exposing the outside object.

Explicit retention uses finite, idempotently renewable Revision leases rather than Page content fields. A lease binds an exact Revision, authorized Scope, holder Principal, reason, and expiration; an expired lease is no longer a protection root. A Runtime may offer bounded routing views of actual collection candidates to a Host semantic worker, but the model selects candidates and reasons only. It does not choose global GC policy or bypass Store authorization and protection closure. Permanent retention, early revocation, and collection remain explicit operator actions rather than silent consequences of an ordinary model decision.

A dry run performs no deletion. A future apply operation MUST recalculate roots inside the write transaction and atomically remove candidate Revisions, candidate-owned compatibility projections or source edges, and expired idempotency records. Implementations advertise planning and application separately through `supportsRevisionRetentionPlanning` and `supportsRevisionRetention`; planning support does not imply collection support.

Retention policy is configured by Host, Page kind, storage budget, and value; models should not choose GC parameters on every write.

## 6. Migration from v0.6

The v0.6 reference database already stored stable `page_id` and exact `revision_id` while its public vocabulary exposed `Ref ~= Page` and `Page ~= Revision`. A v0.7 migration restores those identities, backfills Page metadata and Revision parents, removes intra-Page `supersedes`, normalizes Relations to Page endpoints with exact basis Revisions, converges Summary and Validity updates on stable maintained Pages, rebuilds head-only indexes, and removes identity Refs. An implementation that actually exposes an Alias API may explicitly migrate non-identity Refs instead.

Migration MUST be transactional, idempotent, and preceded by a recoverable backup.
