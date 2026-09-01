# Paged-Context-Protocol (PCP) · v0.8.0-draft

[中文](README.md) | **English**

![Paged-Context-Protocol banner](assets/banner.png)

> **Protocol draft and development preview.** The v0.8 protocol, APIs, and Store format may still change. v0.8 is not compatible with v0.7 Stores; migration requires a new Store populated from tenant-held source material.

PCP is an open protocol with a project-maintained Rust implementation for storing, organizing, and retrieving context that persists across tasks. It represents content as stable Pages and immutable Revisions with Scopes, Relations, and Provenance, then returns bounded results for the current task.

PCP can run in-process as the context layer for one application, or an independent Runtime can let several clients contribute separate Scopes under one Identity. A product may use it for current-task context management, long-term memory, project knowledge, session continuity, or a combination of these. The protocol does not require a standalone daemon, multiple tenants, or a background maintainer, nor does it prescribe how those capabilities are named or presented.

## Design Scope

```text
Tenant-held sources
  -> Identity + Scope                    identity, authorization, association
  -> Page + immutable Revision           stable records and content versions
  -> Relation + Provenance + Summary     organization, evidence, derivation
  -> Search + Read + Projection          bounded retrieval and reading
  -> current working context of a Host or model
```

- **Page and Revision**: A Page is an independently retrievable record; a Revision is an immutable content snapshot. Source records are normally `sealed`, while maintained records may be `revisioned`.
- **Identity, Tenant, and Scope**: One Store, whether embedded in a Host or managed by Runtime, serves one durable Identity. Tenants can read and write only authorized Scopes, and the implementation injects request identity.
- **Relation and Provenance**: Relations connect stable Pages. Relation evidence and derivation refer to exact Revisions. Temporal adjacency or textual similarity does not create a domain relation by itself.
- **Search, Read, and Projection**: Search returns candidates before a caller reads Payload, Summary, Sources, Relations, or History projections. The Host decides which results enter its current context.
- **Maintenance and governance**: An implementation may provide Summaries, Topics, Validity, Relations, lossless packing, and retention. The project-maintained Runtime also provides optional background maintenance and review. Deployment policy determines whether validated low-risk work applies automatically.
- **Content updates and feedback**: Tenants can write new information normally or challenge readable old Revisions, keeping actually-used context separate from new corrective evidence. Maintenance proposes validity or replacement decisions; cross-Scope decisions, replacements, and retractions require Console approval without silently rewriting the original Page.
- **External sources**: Tenants retain and understand their own chat records, media, or domain objects. PCP stores a minimal SourceRef and optional digest, then returns authorized source coordinates. Source parsing, search, and rendering remain tenant responsibilities.

PCP does not prescribe a Router, prompt format, Chain-of-Thought, context-window planner, or model state machine. It defines boundaries for durable records, authorization, sources, retrieval, and optional maintenance operations. SQLite, a standalone Runtime, semantic models, Console interactions, and Host workflows are implementation choices.

## Current Implementation

This repository contains the specification and the project-maintained Rust implementation. An application can compose a Store in-process through the embedded client or use the same objects and tenant contract over `pcp-runtime` RPC. `pcp-runtime` is a reference service profile for local multi-client deployment; it is neither the protocol itself nor a prerequisite for using PCP. Discovery, enrollment, Observer, and background scheduling belong only to that service profile. The workspace also provides a CLI, MCP server, and local Console.

New clients can discover Runtime through [Infra Discovery](https://github.com/glenzli/infra-protocol), request a Principal, access mode, and Scopes, then receive an identity-bound endpoint for the current generation after approval. An approved registration can rediscover Runtime and open a new session after a restart.

![PCP Console showing a local Store overview with synthetic demo data](assets/console-overview.png)

*Local Store overview in PCP Console. The screenshot uses synthetic data and contains no real Page, Scope, or client identity.*

### Implemented

- Stable Pages, immutable Revisions, `sealed`/`revisioned` behavior, and CAS updates.
- `exact`, `text`, `graph`, `temporal`, and `auto` retrieval with bounded Projection reads.
- Runtime-RPC `semantic_search`, `match_intent`, and explicit-anchor graph expansion.
- Summary, Topic, Validity, Relation, Provenance, archive/restore, lossless sealed-Page packing, and access audit.
- Runtime-injected Identity and Actor for `ingest_page`, with optional `sourceSpan`, `basedOnRevisionIds`, and a minimal SourceRef.
- Tenant `submit_feedback`, per-target reconciliation, atomic Validity/`supersedes` commits, and a bounded Luna-to-Sol-to-human escalation path.
- Embedded and RPC clients, approved enrollment, CLI, MCP, Console, a maintenance coordinator, and read-only infrastructure observation.
- Deterministic Revision-retention planning, finite leases, and protected explicit collection.

![PCP Console showing synthetic Pages](assets/console-pages.png)

*The Pages view shows Page kinds, Scopes, source spans, and direct relations. All content is synthetic demo data.*

### Current Boundaries

- Durable Page deletion, cold storage, and Identity-wide Validity maintenance are not implemented; `purge` is outside v0.8.
- External-source custody, parsing, retrieval, rendering, OCR, and transcription belong to tenants.
- Semantic queries require an explicitly configured Provider. An unavailable Provider produces an unavailable result rather than an automatic keyword fallback.
- Local Unix-socket mode `0600` is an OS-user boundary and does not defend against a hostile process running as the same user.
- Public conformance is defined by [`PROTOCOL-en.md`](PROTOCOL-en.md), not by a specific backend or interface in this repository.

## Repository Layout

| Crate | Role |
| --- | --- |
| `pcp-core` | Core objects, requests, projections, and capability types |
| `pcp-store` | Database-independent Store contract with `AccessSession` |
| `pcp-client` | Tenant `PcpTenantApi`, privileged `PcpApi`, and embedded client |
| `pcp-rpc` | Unix-socket wire, remote client, and server transport |
| `pcp-sqlite` | SQLite Store, retrieval, audit, and retention |
| `pcp-runtime` | Identity-bound endpoints, client enrollment, and maintenance coordinator |
| `pcp-cli` | Inspection, retrieval, read, export, and retention operations |
| `pcp-mcp` | Local stdio MCP server |
| `pcp-console` | Local Store Inspector, review, and governance entry point |

## Quick Start

The workspace uses Rust 2024 edition:

```bash
cargo test --workspace

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- doctor

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- retention-plan 30 2 100
```

`retention-plan` is a dry run. Physical collection uses `retention-collect --confirm` and replans exact Revision IDs before submission.

## Deployment

`PcpTenantApi` is the ordinary tenant interface. It exposes the descriptor, authorized Scopes, `ingest_page`, Search, Read, and optional browse. `PcpApi` is the privileged superset for Runtime maintainers and local administration tools. A Host can embed a Store or connect to a separate Runtime:

```text
Tenant Host --> PcpTenantApi --> EmbeddedPcpClient --> PcpStore
                         `-----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex --------> MCP -----------> PcpTenantApi
Runtime/CLI -------------------> PcpApi
```

Start a multi-client Runtime:

```bash
cargo build --release -p pcp-runtime
target/release/pcp-runtime --config examples/runtime.toml
```

Each RPC endpoint binds one Principal injected by Runtime; requests cannot select their own identity. Isolate tenants or model contexts with separate endpoints and minimal Scopes.

### Managed macOS Service

PCP can use its local Console service to manage `pcp-runtime`, the Store, sockets, enrollment state, and maintenance ledger. The default data directory is `~/Library/Application Support/PCP`:

```bash
sh scripts/install-macos.sh
```

LaunchAgent `com.glenzli.pcp-console` starts `pcp-console --managed`. The generated Runtime configuration keeps automatic maintenance disabled by default. After configuring a separately authorized worker, an operator may select observe or apply mode.

An existing Store and enrollment state can be imported before first launch:

```bash
sh scripts/import-store.sh \
  --source /absolute/path/to/context.sqlite3 \
  --enrollment-state /absolute/path/to/pcp-enrollments.json
```

### MCP

MCP can open an embedded Store directly:

```bash
cargo build --release -p pcp-mcp
codex mcp add pcp \
  --env PCP_STORE_PATH=/absolute/path/to/context.sqlite3 \
  --env PCP_CLIENT_ID=codex:project-example \
  --env PCP_ACCESS_MODE=read \
  --env PCP_ALLOWED_SCOPES=project:example,conversation:example-main \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

It can also connect to an identity-bound Runtime:

```bash
codex mcp add pcp \
  --env PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
  --env PCP_CLIENT_ID=codex:project-example \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

Long-running MCP clients should use enrollment instead of persisting a generation-specific Runtime socket. `pcp-mcp enroll begin` creates a mode `0600` local credential state and submits an access request; after Console approval, run `pcp-mcp enroll status` to complete registration. Then pass `PCP_ENROLLMENT_FILE` and the matching `PCP_CLIENT_ID` in MCP configuration. Each process start reopens a session through the current Infra Discovery registration. Static `PCP_RUNTIME_SOCKET` remains only for explicitly configured compatibility endpoints.

An ordinary writable tenant should use `contribute`, which adds authenticated `ingest_page` and exact-Revision `submit_feedback` to Read. `repair` is a narrow development-migration surface for history-preserving `repair_page`; it does not grant ordinary Page writes, revisions, lifecycle changes, or Scope administration. Use a separate Principal and credential, opened only during an explicit apply migration. `write` and `admin` remain reserved for maintainers and local administration tools. See [`crates/pcp-runtime/ENROLLMENT.md`](crates/pcp-runtime/ENROLLMENT.md) for access modes and the enrollment contract.

### Local ChatGPT Access

ChatGPT Developer Mode can call the local stdio `pcp-mcp` through OpenAI Secure MCP Tunnel. PCP Runtime, its Store, Unix sockets, and enrollment credential remain off the public internet; the local tunnel client creates an outbound HTTPS connection to OpenAI. This surface uses a separate `chatgpt:pcp` Principal, `chatgpt-pcp.json` enrollment state, and `chatgpt_capture` Page kind rather than reusing the Codex grant or source label.

`scripts/install-macos.sh` installs the entry point at `~/Library/Application Support/PCP/bin/pcp-chatgpt-mcp`. See [`integrations/chatgpt`](integrations/chatgpt/README.md) for enrollment, tunnel configuration, and Developer Mode connection steps. This is a private development path, not a replacement for the public HTTPS MCP deployment required by a public ChatGPT app.

### Maintenance, Console, and Observation

Background maintenance and manual Console runs use the same persistent review queue. A worker produces candidates; Runtime and Store retain control of budgets, authorization, current-Revision checks, and commits. Relation, Topic, and Archive proposals requiring judgment are reviewed before application. Scheduling, model escalation, and failure backoff are documented in [`crates/pcp-runtime/README.md`](crates/pcp-runtime/README.md).

Console should connect through a dedicated `audit` endpoint. It provides read-only Store inspection, query previews, enrollment management, maintenance review, and authorized archive/restore. Runtime's infrastructure observer returns aggregate, redacted operational data only; see [`crates/pcp-runtime/OBSERVER.md`](crates/pcp-runtime/OBSERVER.md).

```bash
cargo build --release -p pcp-console
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-operator.sock \
PCP_CLIENT_ID=operator:local \
PCP_CONSOLE_BIND=127.0.0.1:4318 \
  target/release/pcp-console
```

## Codex Plugin

[`plugins/pcp`](plugins/pcp) is the source bundle for the Codex plugin. It combines `pcp-mcp`, the [`use-pcp`](plugins/pcp/skills/use-pcp/SKILL.md) Skill, tool approval policy, and icon behind one entry point. It does not bundle Runtime or a Store, and it does not create access grants for the user. The public marketplace snapshot includes a compiled macOS arm64 `pcp-mcp`; other platforms may select a compatible build through `PCP_MCP_BINARY` or use the PCP system installation. `pcp-runtime` and `pcp-console` remain independent local services and must first be installed from a PCP release or from the source repository with `scripts/install-macos.sh`. Then create and approve a `contribute` enrollment for `codex:pcp` as documented by the [enrollment contract](crates/pcp-runtime/ENROLLMENT.md). The plugin opens that enrollment from `~/Library/Application Support/PCP/clients/codex-pcp.json` by default.

Install the public release from Glenzli Marketplace:

```bash
codex plugin marketplace add glenzli/marketplace --ref main
codex plugin add pcp@glenzli-marketplace
```

The plugin uses one approved `codex:pcp` Principal. `user:self` has `contribute` access, while `read_all_scopes` provides read-only access to the other Scopes in the current Store. It exposes bounded retrieval, exact-Revision reads, explicit feedback, and high-threshold capture; every `pcp_capture` and `pcp_submit_feedback` call requires confirmation. Capture is limited to explicit retention requests or confirmed preferences, constraints, decisions, findings, and outcomes that are reusable across tasks. It excludes routine progress, raw conversations, logs, speculation, secrets, and facts that are inexpensive to recover from the repository. Start a new Codex task after installation or update so the new Skill and MCP tools enter the task context.

The release boundary is explicit: the PCP repository owns the Rust, plugin, and Skill sources; `cargo build --release -p pcp-runtime -p pcp-console -p pcp-mcp` produces the local services and MCP artifacts; system installation owns Runtime, Console, Store, LaunchAgent, and enrollment state; the public marketplace contains only the validated plugin snapshot, Skill, launcher, icon, license, and `pcp-mcp` dist for supported platforms. It does not hide Runtime lifecycle inside the Codex plugin process.

## Documentation

- [Current specification](PROTOCOL-en.md)
- [Chinese specification](PROTOCOL.md)
- [Runtime notes](crates/pcp-runtime/README.md)
- [Enrollment contract](crates/pcp-runtime/ENROLLMENT.md)
- [Observer contract](crates/pcp-runtime/OBSERVER.md)
- [Historical generations and deprecation rationale](deprecated/README.md)
- [MIT License](LICENSE)
