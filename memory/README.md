# PCP-Native Memory Profile

This directory defines the PCP-native Memory profile, previously explored as
`glenzli/paged-memory`.

Paged Memory is not a separate context protocol. It is the same-origin
persistence profile for PCP: a Memory system that stores and serves PCP Logical
Pages with compatible IDs, manifests, trust labels, evidence resolution, and
Fetch semantics.

## Status

Draft. The goal of this directory is to keep Memory schema and PCP protocol
semantics evolving together while preserving a clear boundary between:

- PCP core protocol: `../PROTOCOL.md`
- English PCP core protocol: `../PROTOCOL-en.md`
- PCP-native Memory profile: `./SPEC.md`

## Why It Lives Here

PCP-native Memory depends on PCP's Page model:

- `Original` and `Consolidated` Pages
- `id`, `source_ids`, `source_ref`, and `source_spans`
- `trust` and provenance chains
- `content_mode` and `available_modes`
- `Query`, `Fetch`, and `Consult` resolution semantics

Keeping Memory in this repository prevents drift between the protocol and the
same-origin storage system that implements long-term page residency.

## Implementation Boundary

The profile is allowed to define storage contracts, query/fetch interfaces,
write-back behavior, and maintenance jobs. It should not redefine the PCP
runtime state machine.

Non-native stores, such as ordinary file indexes, search engines, vector
databases, or external RAG systems, should connect through adapters that wrap
their results into PCP-compatible `Original Page` records.

## Documents

- [SPEC.md](./SPEC.md): PCP-native Memory profile draft.
