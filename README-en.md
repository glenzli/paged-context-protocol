# Paged-Context-Protocol (PCP) - v0.7.0-draft

[中文](README.md) | **English**

![Paged-Context-Protocol banner](assets/banner.png)

An open protocol and its official implementation for deciding what user-owned
information enters model attention, when it enters, and with what identity and
evidence.

## Overview

Frontier models can search files, call tools, and manage long active contexts,
yet continuity across sessions, projects, and models is still commonly reduced
to coarse summaries or closed product Memory. For long-lived, high-density work,
the problem is not only whether information can be stored, but whether the right
material can be found, traced, and selectively admitted into finite attention.

**PCP defines a user-owned, model-independent information space and the paging
boundary between that space and model attention.** It is not a particular context
manager, Memory product, or Storage format, and it does not require all history to
be packed into one prompt. Context is an information continuum across time; the
model's active window is only a temporary working set.

```text
Source / Event
  -> stable Page + immutable Revision       (identity and storage)
  -> Relation / Provenance / Summary        (organization and evidence)
  -> Search / Read / Projection             (selection and materialization)
  -> Active working context                 (model attention)
```

## Protocol Core

- **Page and Revision**: A Page is a stable semantic object; a Revision is an
  immutable content snapshot. Raw Pages are normally `sealed`, while maintained
  Pages may be `revisioned`.
- **Scope and Access**: A unified address space is not global injection. Search,
  read, derivation, and write operations remain constrained by Scope and a
  server-injected access identity.
- **Relation and Provenance**: Relations connect stable Pages, while relation
  evidence and provenance refer to exact Revisions. Temporal adjacency or textual
  similarity does not automatically become a domain edge.
- **Summary and Validity**: Summaries are optional, sparse, traceable derived Pages
  rather than a mandatory tier for every item. Validity records whether information
  remains applicable.
- **Search, Read, and Projection**: Retrieval returns identifiable candidates before
  selected projections such as Summary, Payload, Sources, Relations, or History are
  read. The model or Host chooses the query path and admission timing.
- **Consolidation and Retention**: Multiple current semantic Pages may be explicitly
  contracted into a canonical Page. Historical Revisions are collected only when
  dependencies, leases, and retention rules permit.

PCP does not prescribe a fixed Router, Intent Focus, zoom hierarchy,
Chain-of-Thought, XML flow, or model state machine. Decisions to write, search,
summarize, consolidate, and materialize remain Model Client or Host policy; the
protocol supplies interoperable, traceable, and constrained objects and interfaces.

## Current Status

`v0.7.0-draft` is the current protocol draft. It restores stable Page identity
above immutable Revisions and defines explicit Scope, Provenance, Relation,
sparse Summary, Validity, consolidation, and Revision-retention semantics.

### Official Implementation

This repository is both the canonical home of the PCP specification and the
project-maintained official Rust implementation. It covers the Store, embedded
and remote clients, Unix-socket RPC, Runtime, automatic discovery and approved
client enrollment, CLI, MCP, Console, maintenance, and observation as an
end-to-end deployable PCP system.

PCP remains an open protocol that permits independent implementations. “Official”
means that this implementation is maintained and released by the PCP project; it
does not make SQLite, one retrieval algorithm, one model, or a Host workflow
normative. Conformance is defined by [`PROTOCOL-en.md`](PROTOCOL-en.md).

![PCP Console showing a local Store overview with synthetic demo data](assets/console-overview.png)

> Actual PCP Console interface shown with synthetic demo data. No real Page content,
> Scope, or client identity is included.

| Crate | Role |
| --- | --- |
| `pcp-core` | Core objects, requests, projections, and capability types |
| `pcp-store` | Database-independent Store contract with `AccessSession` |
| `pcp-client` | Transport-independent `PcpApi` and embedded client for Hosts |
| `pcp-rpc` | Local Unix-socket wire, remote client, and server transport |
| `pcp-sqlite` | SQLite Page/Revision Store, migrations, retrieval, audit, and retention |
| `pcp-runtime` | Identity-bound endpoints, approved client enrollment, and optional maintenance coordinator |
| `pcp-cli` | Inspection, retrieval, read, export, consolidation, and retention operations |
| `pcp-mcp` | Local stdio tool server built on the official Rust MCP SDK |
| `pcp-console` | Independent read-only local Web Inspector |

### Implemented

- Stable Pages, immutable Revisions, `sealed`/`revisioned` behavior, CAS updates,
  and idempotent migration from v0.6 data.
- Head-only default retrieval, `auto`, `exact`, `text`, `graph`, and `temporal`
  modes, plus bounded Projection reads.
- Summary, Validity, Relation, Provenance, consolidation, and access audit.
- Identity-bound embedded and RPC clients, discoverable user-approved Runtime
  enrollment, CLI, MCP, and Console.
- Deterministic Revision-retention planning, finite leases, protected explicit
  collection, and multidimensional Health diagnostics.

### Not Yet Implemented

Aliases and durable Page deletion are currently reported as unavailable through
Capabilities, and cold storage is not yet implemented.

### Implementation Boundary

The official implementation intentionally does not embed a semantic model or
Router. Deployments may compose local models, remote models, full-text retrieval,
or other strategies above the protocol interface without changing PCP object,
authorization, or provenance semantics.

## Quick Start

The workspace currently uses Rust 2024 edition:

```bash
cargo test --workspace

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- doctor

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- retention-plan 30 2 100
```

`retention-plan` is a dry run. Its arguments are minimum age in days, recent
Revisions retained per Page, and candidate limit. Physical collection uses the
separate `retention-collect --confirm` command and replans exact Revision IDs
before submission.

## Deployment

`PcpApi` is the consumer boundary. A Host may embed a Store directly or use the
optional Runtime for an independent lifecycle and fixed server-side identities.
Both CLI and MCP can connect through either shape.

```text
Host --------> PcpApi --> EmbeddedPcpClient --> PcpStore
                    `----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex -------> MCP -----> PcpApi
Operator ----> CLI -----> PcpApi
```

For multiple clients, start the broker from
[`examples/runtime.toml`](examples/runtime.toml). These static endpoints may
remain available while clients migrate to automatic discovery and enrollment:

```bash
cargo build --release -p pcp-runtime
target/release/pcp-runtime --config examples/runtime.toml
```

Each Unix socket maps to one fixed Principal injected by Runtime; requests cannot
choose their own identity. Socket mode is `0600`. This is a local-user boundary,
not protection from a hostile process already running as the same OS user. Strong
isolation requires separate endpoints, minimal Scopes, and separate model contexts,
because Storage authorization cannot retract information already visible to a model.

Runtime also advertises approved client enrollment through
[Infra Discovery](https://github.com/glenzli/infra-protocol). A local client
requests a Principal, access mode, and Scopes; after approval in Console it
receives an identity-bound RPC endpoint for the current generation. Following a
Runtime restart, it rediscovers and reopens the durable registration instead of
relying on a hard-coded socket path.

See [`crates/pcp-runtime/ENROLLMENT.md`](crates/pcp-runtime/ENROLLMENT.md) for the
complete contract and [Symbiont](https://github.com/glenzli/symbiont-d) migration
sequence.

### Runtime Maintenance

The maintenance coordinator is optional and defaults to observation without
applying changes. A configured semantic worker may return only `write_summary`,
`consolidate`, `keep_separate`, or `defer` decisions over bounded candidates and
Detail. It cannot write the Store directly; Runtime supplies mechanical metadata,
and the Store revalidates authorization, current heads, lineage, and atomicity.
The maintainer never performs automatic Revision collection.

See [`crates/pcp-runtime/README.md`](crates/pcp-runtime/README.md).

### Infrastructure Observation

Runtime advertises a read-only observer capability through
[Infra Discovery](https://github.com/glenzli/infra-protocol).
[Infra Sentinel](https://github.com/glenzli/infra-sentinel) reads a versioned,
aggregate-only, redacted snapshot over an owner-only Unix socket: uptime,
24-hour calls, failures and denials, current Page count, and optional p95 latency
and telemetry coverage. Page content, queries, Scope names, raw audit, and
maintenance actions are excluded. Console is only an optional PCP snapshot deep
link, not part of discovery or the observer data interface.

See [`crates/pcp-runtime/OBSERVER.md`](crates/pcp-runtime/OBSERVER.md) for the
contract and Python wire example.

### Codex / MCP

MCP can open an embedded SQLite Store directly:

```bash
cargo build --release -p pcp-mcp
codex mcp add pcp \
  --env PCP_STORE_PATH=/absolute/path/to/context.sqlite3 \
  --env PCP_CLIENT_ID=codex:project-example \
  --env PCP_ACCESS_MODE=read \
  --env PCP_ALLOWED_SCOPES=project:example,conversation:example-main \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

Alternatively, MCP can connect to an identity-bound Runtime:

```bash
codex mcp add pcp \
  --env PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
  --env PCP_CLIENT_ID=codex:project-example \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

`PCP_ACCESS_MODE` may be `observe`, `read`, `audit`, `write`, or `admin`. `observe`
can read aggregate Health only; it cannot list or read Pages, search, read raw
audit events, or invoke maintenance actions. Cross-Scope derivation still requires
a separate opt-in even in `admin` mode. `pcp_whoami` reports the server-injected
Principal and grants. Read tools do not mutate content; Page, Summary, Relation,
Scope, and Validity tools are marked as writes for MCP approval.

### Console

The Console should use a dedicated `audit` endpoint. Its Store Inspector is
read-only and exposes Page, Relation, access-timeline, Retention, and Health
views; its only control-plane actions approve, reject, or revoke local client
registrations. Health presents storage shape, activity, recall, consolidation,
graph, and operations separately rather than as an opaque score. Operational
telemetry excludes query text and Page content.

```bash
cargo build --release -p pcp-console
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-operator.sock \
PCP_CLIENT_ID=operator:local \
PCP_CONSOLE_BIND=127.0.0.1:4318 \
  target/release/pcp-console
```

Console and Runtime default to `pcp-enrollment-admin.sock` beside their static
endpoint. If the operator endpoint and the broker's first endpoint use different
directories, set the same absolute `PCP_ENROLLMENT_ADMIN_SOCKET` for both.

## Specification and History

- Current specification: [PROTOCOL-en.md](PROTOCOL-en.md)
- Chinese specification: [PROTOCOL.md](PROTOCOL.md)
- Runtime notes: [crates/pcp-runtime/README.md](crates/pcp-runtime/README.md)
- PCP Runtime observer contract: [crates/pcp-runtime/OBSERVER.md](crates/pcp-runtime/OBSERVER.md)
- PCP Runtime enrollment contract: [crates/pcp-runtime/ENROLLMENT.md](crates/pcp-runtime/ENROLLMENT.md)
- Historical generations and deprecation rationale: [deprecated/](deprecated/README.md)
- License: [MIT](LICENSE)
