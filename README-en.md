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

With background maintenance enabled, related same-Scope candidates are organized into memory drafts that preserve chronology, disagreement and sources. One group may produce multiple memories or propose an update to an existing Page; with candidate automatic review enabled in Console, maintenance apply mode and the shared review budget enabled, drafts settle for about five minutes before Sol assesses each output. Approved outputs are written automatically while unresolved evidence remains. Repaired drafts receive independent verification; Console can pause automatic review, inspect evidence and withdraw automatic writes. New evidence coalesces for about two minutes, with at most one bounded organization job per cycle and no repeated model calls for unchanged evidence. Unresolved candidates are retained; only resolved operational receipts expire. See [candidate evolution](design/candidate-evolution.md) for review and recovery boundaries.

![PCP Console showing a local Store overview with synthetic demo data](assets/console-overview.png)

*Local Store overview in PCP Console. The screenshot uses synthetic data and contains no real Page, Scope, or client identity.*

### Implemented

- Stable Pages, immutable Revisions, `sealed`/`revisioned` behavior, and CAS updates.
- `exact`, `text`, `graph`, `temporal`, and `auto` retrieval with bounded Projection reads.
- Runtime-RPC `semantic_search`, `match_intent`, and explicit-anchor graph expansion.
- Summary, Topic, Validity, Relation, Provenance, archive/restore, lossless sealed-Page packing, and access audit.
- Runtime-injected Identity and Actor for `ingest_page`, with optional `sourceSpan`, `basedOnRevisionIds`, and a minimal SourceRef.
- Tenant `submit_feedback`, per-target reconciliation, atomic Validity/`supersedes` commits, and a bounded Luna-to-Sol-to-human escalation path.
- A Runtime-local Context Inbox with explicitly enabled candidate staging and short-lived activity cards; formal promotion requires operator approval or the enabled shared-budget Sol review.
- Embedded and RPC clients, approved enrollment, CLI, MCP, Console, a maintenance coordinator, and read-only infrastructure observation.
- Deterministic Revision-retention planning, finite leases, and protected explicit collection.

![PCP Console showing synthetic Pages](assets/console-pages.png)

*The Pages view shows Page kinds, Scopes, source spans, and direct relations. All content is synthetic demo data.*

### Current Boundaries

- Durable Page deletion, cold storage, and Identity-wide Validity maintenance are not implemented; `purge` is outside v0.8.
- External-source custody, parsing, retrieval, rendering, OCR, and transcription belong to tenants.
- Semantic queries require an explicitly configured Provider. An unavailable Provider produces an unavailable result rather than an automatic keyword fallback.
- The Context Inbox does not aggregate independent Stores or confirm and promote candidates from repeated mentions across clients.
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

Upgraded Infer Runtime reviews are configured under `[maintenance.worker.review_budget]` and are disabled by default. Sol High defaults to 300 calls / 6 million total tokens per rolling 24 hours; Astra Low defaults to 10 calls. Console exposes limits, actual usage, and unsettled reservations; restarting preserves the ledger. The old unbudgeted escalation path is disabled. Budgets gate new calls using actual usage and reservations. Admitted calls finish normally, so the final call may exceed the token threshold. See the [review budget and repair workflow](design/maintenance-review-budget.md).

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

### Shared ChatGPT and Codex tunnel access

Use one PCP connection created in ChatGPT from both ChatGPT and Codex. The path is ChatGPT / Codex → OpenAI Secure MCP Tunnel → local `pcp-chatgpt-mcp` → PCP Runtime → Store. The tunnel connects outbound over HTTPS; no public listener is required on the Mac. Tool requests and returned content still pass through OpenAI. Runtime, Store, and enrollment files remain locally managed.

#### 1. Install and start PCP

From this repository, with a Rust toolchain installed:

```bash
sh scripts/install-macos.sh
```

This builds and installs Runtime, Console, `pcp-mcp`, and the `pcp-chatgpt-mcp` launcher. Open [PCP Console](http://127.0.0.1:4318/) and confirm Runtime is connected. Install the tunnel client separately.

#### 2. Enroll the shared PCP identity

For a new installation, select the current Infra Discovery registration manifest named `pcp--idn_....json` for the intended Store. Do not reuse an old socket or select another instance by guesswork; see the [enrollment contract](crates/pcp-runtime/ENROLLMENT.md) for discovery and verification.

List the default macOS Discovery directory below. If Runtime sets `INFRA_PROTOCOL_RUNTIME_DIR`, use its `registrations` directory instead. With multiple results, match JSON `service.instance_id` to the Store identity in Console.

```bash
ls "$(getconf DARWIN_USER_TEMP_DIR)infra-protocol/registrations/"pcp--*.json
```

```bash
pcp_home="$HOME/Library/Application Support/PCP"
"$pcp_home/bin/pcp-chatgpt-mcp" enroll begin \
  "/absolute/path/to/current/pcp--idn_....json"
```

Review and approve `ChatGPT` in Console client access, then run:

```bash
"$pcp_home/bin/pcp-chatgpt-mcp" enroll status
```

The state file is `clients/chatgpt-pcp.json`. The `chatgpt:pcp` Principal requests `contribute` on the user Scope and read-only access to other current Scopes. Reuse an existing working ChatGPT enrollment and tunnel. Codex shares this identity; the historical `chatgpt_capture` / `captureSurface: chatgpt` labels identify the connection, not which application called it.

#### 3. Configure and test the tunnel

Follow [OpenAI Secure MCP Tunnel setup](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels) to create a tunnel in Platform, obtain its ID and runtime API key, and install the official `tunnel-client`. Associate the intended ChatGPT workspace and grant the operator tunnel usage; permission to create a tunnel alone does not establish workspace access.

With `tunnel-client` on PATH, replace `YOUR_TUNNEL_ID` below and enter the key privately in the local macOS zsh terminal:

```bash
read -r -s 'CONTROL_PLANE_API_KEY?Tunnel runtime API key: '
export CONTROL_PLANE_API_KEY

tunnel-client init \
  --sample sample_mcp_stdio_local \
  --profile pcp-chatgpt \
  --tunnel-id YOUR_TUNNEL_ID \
  --mcp-command "\"$HOME/Library/Application Support/PCP/bin/pcp-chatgpt-mcp\""
tunnel-client doctor --profile pcp-chatgpt --explain
tunnel-client run --profile pcp-chatgpt
```

Keep this process running. Enable Developer Mode in ChatGPT, create a connection named `PCP` from Plugins, and select **Tunnel** with the same ID. Review discovered tools and write-action permissions. Account and workspace policy affect availability; see the [current connection instructions](https://developers.openai.com/plugins/deploy/connect-chatgpt).

#### 4. Run at login

After a successful foreground test, run this in a second terminal:

```bash
python3 integrations/chatgpt/install-service.py
```

Defaults are `~/Applications/tunnel-client/tunnel-client` and `~/.config/tunnel-client/pcp-chatgpt.yaml`. Override them with `--binary /absolute/path/to/tunnel-client` and `--profile /absolute/path/to/profile.yaml`. Without the environment variable, the installer prompts for the key without echoing it. The saved key has mode `0600`; the LaunchAgent references its path.

Wait for readiness and a completed control-plane poll, then stop the old foreground process. Keep only one poller running for the tunnel. The service starts at login and restarts after exit; it cannot serve local PCP while the Mac is asleep, offline, or powered off.

```bash
launchctl print "gui/$(id -u)/com.glenzli.pcp-chatgpt-tunnel"
# Load an updated pcp-mcp binary into the tunnel child process:
launchctl kickstart -k "gui/$(id -u)/com.glenzli.pcp-chatgpt-tunnel"
```

Inspect the local [tunnel status UI](http://127.0.0.1:4319/ui). See the [full integration guide](integrations/chatgpt/README.md) for private file locations, logs, key rotation, and shutdown.

#### 5. Verify the shared entry point

Enable the same PCP connection in a new ChatGPT conversation and a new Codex task. Do not install a local PCP Codex plugin. Retrieve a known topic and inspect its sources, then call `pcp_whoami` (included in the default compact toolset): both should report `chatgpt:pcp`, the same user Scope, and matching grants. Old tasks may retain their previously loaded tool context.

If discovery fails, check Runtime in Console, enrollment, tunnel readiness and polling, then workspace association. Passing `doctor` does not prove a running service or a successful conversation call. Refresh connection metadata after changing tools. Host action controls govern confirmation for capture and feedback; check them after updates or reconnecting.

### Maintenance, Console, and Observation

Background maintenance and manual Console runs use the same persistent review queue. A worker produces candidates; Runtime and Store retain control of budgets, authorization, current-Revision checks, and commits. Ordinary Relations can opt into independent full-source verification before automatic application; uncertain relations and Archive proposals require review. Store-wide maintenance can periodically revisit old Pages and synthesize Topics across authorized Scopes; accumulated short Pages can qualify without meeting the long-Page summary threshold. Topic auto-application is a separate deployment opt-in and requires independent full-text verification of new information and preserved qualifications. Exact source revisions, neighboring topics, and recorded rejection reasons help avoid repeat proposals. Invalid candidates receive one correction attempt before isolation; a pending Topic limit pauses new proposals. Scheduling, model escalation, and failure backoff are documented in [`crates/pcp-runtime/README.md`](crates/pcp-runtime/README.md).

Console should connect through a dedicated `audit` endpoint. It provides read-only Store inspection, query previews, enrollment management, maintenance review, and authorized archive/restore. Runtime's infrastructure observer returns aggregate, redacted operational data only; see [`crates/pcp-runtime/OBSERVER.md`](crates/pcp-runtime/OBSERVER.md).

The Console access timeline counts completed Runtime API requests by default. Runtime-generated request IDs correlate internal operations, whose details load in pages only when expanded. Background maintenance has an explicit origin; older records remain in the uncorrelated view without timestamp-based grouping. Cross-scope request aggregates require audit access to every contributing scope. Semantic search re-enumerates authorized current revisions on each query and skips body reads for revisions already cached in the same embedding space.

```bash
cargo build --release -p pcp-console
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-operator.sock \
PCP_CLIENT_ID=operator:local \
PCP_CONSOLE_BIND=127.0.0.1:4318 \
  target/release/pcp-console
```

## Why there is no separate Codex plugin

PCP maintains one ChatGPT connection and reuses it in Codex. Both already access the same Store, and the persistent tunnel is already needed for ChatGPT. A separate Codex package adds installation, versioning, enrollment, and guidance to maintain, while exposing duplicate tools in the same task. Sharing the connection removes that ambiguity; client attribution alone does not justify a second entry point.

Both applications now depend on the same connector and tunnel path. If it is unavailable, both temporarily lose PCP access. Normal use requires no additional service because the tunnel already runs continuously. The shared identity retains its existing Scope grants; application names do not confer extra permissions.

This repository no longer distributes a standalone Codex plugin, dedicated Skill, or marketplace snapshot. It continues to maintain `pcp-mcp`, Runtime, Console, and the local tunnel launcher. Generic stdio MCP remains available for development and troubleshooting. MCP instructions, tool descriptions, and documentation carry the policy: retrieve when context matters, read exact sources, and retain a high threshold for formal writes. With activity enabled, read when starting or resuming substantive topics and update goals, milestones, next steps, blockers, pauses, and completion as they change; skip unchanged writes and per-message logs.

To migrate, verify the shared connection, uninstall the local PCP plugin in Codex, and start a new task. Existing Store data, memories, ChatGPT enrollment, and the tunnel require no migration or recreation. An unused old Codex enrollment may be revoked separately in Console. Private tunnel access is not publication to a public plugin directory.

## Documentation

- [Model-facing tools and compact responses](integrations/TOOL_INTEGRATION.md)

- [Current specification](PROTOCOL-en.md)
- [Chinese specification](PROTOCOL.md)
- [Runtime notes](crates/pcp-runtime/README.md)
- [Enrollment contract](crates/pcp-runtime/ENROLLMENT.md)
- [Observer contract](crates/pcp-runtime/OBSERVER.md)
- [Historical generations and deprecation rationale](deprecated/README.md)
- [MIT License](LICENSE)
