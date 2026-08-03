# Paged-Context-Protocol (PCP) - v0.6.0-draft

![Paged-Context-Protocol banner](assets/banner.png)

A model-agnostic protocol for paging user-owned information across persistent
Storage, model Memory, and active Context.

一个模型无关的上下文分页协议，让用户拥有的信息在持久 Storage、模型 Memory 与 active
Context 之间保持统一身份、可追溯组织和按需物化。

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 简介 (Chinese)

现代模型能够搜索文件、调用工具并管理当前窗口，但它们通常仍被当前会话和项目边界限制。
完整 Storage 太大，无法直接进入注意力；传统 Memory 往往只保留粗糙压缩，又容易丢失来源、
版本和适用范围。

**Paged-Context-Protocol (PCP)** 定义一个由用户拥有的不可变 Page 图。原始事件、文件、
历史讨论、模型总结和跨项目关系都以 Page 存在，并按需进入模型注意力。
这里的 Context 是跨时间存在的信息连续体；模型当前窗口只是其中一个临时 working set。

PCP 不是特定的 context manager、Memory 数据库或 Storage 格式。它定义三者之间可互操作、
可追溯的分页边界：

```text
Source / Event
  -> immutable Page              (Storage)
  -> Relation / optional Ref     (Memory organization)
  -> Search / Read               (Attention boundary)
  -> Active working context
```

其中 Memory 表示 PCP 内部的派生组织层，不要求部署独立的 Memory 服务或 Profile。

### 核心原则

- **用户拥有信息连续性**：历史不会绑定在单一模型、服务商、会话或项目目录中。
- **不可变内容身份**：Page 创建后不原地修改；变化由新 Page 与 `supersedes` 表达。
- **原始历史可恢复**：在授权范围内保留可搜索来源；摘要和模型整理是可重建的派生层。
- **稀疏模型记忆**：只有值得索引的内容才创建 Summary Page，模型再按需读取目标 Page。
- **可回溯整理图**：总结、聚合、判断与后继关系都能逐层返回精确来源 Page。
- **显式关系断言**：时间相邻和内容相似不会自动成为 Relation；领域关系由 Host、用户或模型判断。
- **显式注意力物化**：已返回候选与读取内容必须以明确 Page 身份进入模型可见上下文。
- **显式作用域**：统一地址空间不等于全局注入，跨项目召回必须由 Scope、权限或关系允许。
- **模型拥有策略**：搜索计划、排序、压缩和具体准入时机仍由模型或 Host 决定。

### PCP 不规定什么

PCP 不规定四处理器、Intent Focus、固定四级变焦、XML Linear Flow、Chain-of-Thought 或
`Consult/Explore/Shelve/Purge` 状态机。它定义 Page、Scope、Provenance、Relation、可选 Ref、
Summary Page、attention materialization 和基础 I/O 语义，
但不规定模型必须采用哪条召回路径，也不替模型决定某项内容何时最值得进入注意力。

### 当前状态

`v0.6.0-draft` 将旧 Page/Revision 双层模型收敛为不可变 Page，并把 Summary 与有效性判断
统一为派生 Page。参考实现包含从旧数据结构幂等升级的迁移。

旧 `v0.3.0-alpha` 作为一次重要的设计阶段被完整保留，并附有淘汰原因和迁移说明：
[deprecated/v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md)。

---

<a name="english"></a>
## Introduction (English)

Modern models can search files, call tools, and manage an active window, but
their visibility usually still ends at the current session and project. Complete
Storage is too large to place directly into attention, while conventional Memory
often preserves only coarse compression and loses source, revision, or scope.

**Paged-Context-Protocol (PCP)** defines a user-owned graph of immutable Pages.
Raw events, files, past discussions, model summaries, and cross-project
Relations can share stable identities and enter model attention through
different Projections on demand. Context here is an information continuum across
time; the model's current window is only one temporary working set.

PCP is not a specific context manager, Memory database, or Storage format. It
defines an interoperable and traceable paging boundary among them:

```text
Source / Event
  -> immutable Page              (Storage)
  -> Relation / optional Ref     (Memory organization)
  -> Search / Read               (Attention boundary)
  -> Active working context
```

Here, Memory names PCP's derived organization layer; it does not require a
separate Memory service or profile.

### Core Principles

- **The user owns information continuity**: history is not bound to one model,
  provider, session, or project directory.
- **Immutable content identity**: Pages are never edited in place; a new Page
  and `supersedes` express change.
- **Recoverable raw history**: authorized sources remain searchable; summaries
  and model organization are rebuildable derived layers.
- **Sparse model memory**: only content worth indexing receives a Summary Page;
  models then read exact target Pages on demand.
- **Traceable organization graph**: summaries, aggregates, assessments, and
  successor chains lead back to exact source Pages.
- **Explicit Relation assertions**: temporal adjacency and similarity do not
  become graph edges automatically; domain Relations require Host, user, or
  model judgment.
- **Explicit attention materialization**: returned candidates and reads enter
  model-visible context with explicit Page identities.
- **Explicit scope**: a unified address space is not global injection;
  cross-project recall requires Scope, permission, or explicit Relations.
- **The model owns strategy**: search planning, ranking, compaction, and exact
  admission timing remain Model Client or Host policy.

### What PCP Does Not Prescribe

PCP no longer mandates four processors, Intent Focus, fixed zoom levels, an XML
Linear Flow, Chain-of-Thought, or a `Consult/Explore/Shelve/Purge` state machine.
It defines Page, Scope, Provenance, Relation, optional Refs, Summary Pages,
Detail reads, attention materialization, and basic I/O semantics without
prescribing a recall path or deciding when a specific item is most worthy of
model attention.

### Current Status

`v0.6.0-draft` collapses the old Page/Revision model into immutable Pages and
represents Summaries and validity assessments as derived Pages. The reference
implementation includes an idempotent migration from the earlier layout.

The complete `v0.3.0-alpha` generation is preserved as a significant design
stage, together with its deprecation rationale and migration notes:
[deprecated/v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md).

---

## 参考实现 / Reference Implementation

仓库包含一个正在演进的 Rust 参考实现。它用于验证协议边界，不把某一种数据库、检索策略或
Host 行为变成协议要求：

- `pcp-core`：Page、Relation、Ref 语义与请求类型。
- `pcp-store`：与具体数据库无关、携带 AccessSession 的异步 Store 契约。
- `pcp-client`：面向 Host 的传输无关能力接口，以及当前的 embedded 适配器。
- `pcp-rpc`：轻量 Unix socket wire、远端 client 与通用 server transport。
- `pcp-console`：通过专用审计 Principal 运行的只读本地 Web Inspector。
- `pcp-sqlite`：本地不可变 Page、检索、Summary、有效性、Ref 与 DAG 关系。
- `pcp-runtime`：打开 Store、固定接入身份并管理多端点 broker 生命周期。
- `pcp-cli`：面向本地 Store 的检查、搜索、读取与导出工具。
- `pcp-mcp`：基于官方 Rust MCP SDK 的本地 stdio 工具服务器。

The repository includes an evolving Rust reference implementation. It exercises
the protocol boundary without making one database, retrieval policy, or Host
behavior normative:

- `pcp-core`: Page, Relation, Ref semantics, and request types.
- `pcp-store`: a database-independent async Store contract with AccessSession
  enforcement.
- `pcp-client`: the transport-independent Host API and the current embedded
  adapter.
- `pcp-rpc`: the lightweight Unix socket wire, remote client, and generic
  server transport.
- `pcp-console`: a read-only local Web Inspector using a dedicated audit
  Principal.
- `pcp-sqlite`: immutable local Pages, retrieval, Summaries, validity, Refs,
  and DAG Relations.
- `pcp-runtime`: Store composition, fixed endpoint identities, and the
  multi-endpoint broker lifecycle.
- `pcp-cli`: inspection, search, read, and export commands for a local Store.
- `pcp-mcp`: a local stdio tool server built on the official Rust MCP SDK.

```bash
cargo test --workspace
PCP_STORE_PATH=data/context.sqlite3 cargo run -p pcp-cli -- doctor
```

模型如何决定写入、召回、总结或让信息进入注意力，仍属于 Model Client 或 Host 策略。
The decision to write, recall, summarize, or admit information into attention
remains Model Client or Host policy.

### Deployment shapes

PCP is a composable capability framework, not one mandatory client or daemon.
`PcpApi` is the consumer boundary. `EmbeddedPcpClient` binds an AccessSession
directly to a `PcpStore`; the lightweight `pcp-rpc` crate provides
`RemotePcpClient` over a local Unix socket without linking SQLite or the daemon
composition. CLI and MCP can use either shape without defining separate storage
behavior:

```text
Host --------> PcpApi --> EmbeddedPcpClient --> PcpStore
                    `----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex -------> MCP -----> PcpApi
Operator ----> CLI -----> PcpApi
```

The runtime is optional. It gives one Store an independent lifecycle and keeps
AccessSessions on the server side; RPC requests never carry a caller-chosen
Principal. A single socket is one fixed trust identity. One broker process can
share one Store across several separately configured sockets for symbiont-d,
Codex, or other clients. Socket mode is `0600`; this is a local user boundary,
not protection against a hostile process already running as the same OS user.

For multiple clients, start the broker from a TOML file. Relative Store and
socket paths resolve from the config location, and `{owner_id}` inside a Scope
is replaced after the Store opens:

```bash
cargo build --release -p pcp-runtime
target/release/pcp-runtime --config examples/runtime.toml
```

See [`examples/runtime.toml`](examples/runtime.toml) for separate symbiont-d and
Codex endpoints. Duplicate socket paths and Principal IDs, empty Scope sets,
unknown fields, and unsupported access modes fail startup. If any endpoint
cannot start or later exits, the broker stops the remaining endpoints instead
of continuing in a partially available state.

The original environment form remains available for one identity-bound
endpoint:

```bash
cargo build --release -p pcp-runtime -p pcp-cli -p pcp-mcp
PCP_STORE_PATH=/absolute/path/to/context.sqlite3 \
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
PCP_CLIENT_ID=codex:project-example \
PCP_CLIENT_TYPE=model_client \
PCP_ACCESS_MODE=read \
PCP_ALLOWED_SCOPES=project:example,conversation:example-main \
  target/release/pcp-runtime
```

### Read-only Console

Give the Console a distinct endpoint with `access_mode = "audit"`. Audit mode
can list, search, and read Pages and access events, but cannot write, revise,
link, retract, or manage Scopes:

```toml
[[endpoints]]
socket_path = "../run/pcp-operator.sock"
client_id = "operator:local"
client_type = "service"
access_mode = "audit"
allowed_scopes = ["user:{owner_id}", "project:example"]
```

Then run the independent local Console:

```bash
cargo build --release -p pcp-console
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-operator.sock \
PCP_CLIENT_ID=operator:local \
PCP_CONSOLE_BIND=127.0.0.1:4318 \
  target/release/pcp-console
```

The first version is intentionally read-only. Its default view summarizes the
runtime and available Scopes. Page and access lists are cursor-paginated; a Page
opens on its Summary or bounded preview, while full Detail and a navigable
one-hop Relation graph are loaded only when requested. It also exposes the
metadata-only access timeline.

The CLI uses that endpoint when `PCP_RUNTIME_SOCKET` is set. Supplying
`PCP_CLIENT_ID` additionally verifies that the endpoint exposes the expected
Principal:

```bash
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
PCP_CLIENT_ID=codex:project-example \
  target/release/pcp describe
```

### Codex / MCP

Build the local stdio server, then opt in with one explicit client identity,
access mode, and Scope set:

```bash
cargo build --release -p pcp-mcp
codex mcp add pcp \
  --env PCP_STORE_PATH=/absolute/path/to/context.sqlite3 \
  --env PCP_CLIENT_ID=codex:project-example \
  --env PCP_ACCESS_MODE=read \
  --env PCP_ALLOWED_SCOPES=project:example,conversation:example-main \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

Alternatively, point MCP at an already running identity-bound runtime. In this
mode the runtime owns Store and access configuration; MCP verifies its
Principal before exposing tools:

```bash
codex mcp add pcp \
  --env PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
  --env PCP_CLIENT_ID=codex:project-example \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

The equivalent project or user configuration is:

```toml
[mcp_servers.pcp]
command = "/absolute/path/to/paged-context-protocol/target/release/pcp-mcp"
env = { PCP_STORE_PATH = "/absolute/path/to/context.sqlite3", PCP_CLIENT_ID = "codex:project-example", PCP_ACCESS_MODE = "read", PCP_ALLOWED_SCOPES = "project:example,conversation:example-main" }
default_tools_approval_mode = "writes"
```

`PCP_ACCESS_MODE` is `read`, `audit`, `write`, or `admin`. `audit` adds access
log visibility to read-only data permissions. Cross-Scope derivation remains
disabled even in admin mode; a trusted integration must separately set
`PCP_ALLOW_CROSS_SCOPE_DERIVATION=1`. `pcp_whoami` shows the server-injected
Principal and grants. `pcp_access_log` returns metadata-only events when the
session has Audit permission.

For strong isolation, use separate MCP entries and model contexts instead of a
single `write` client spanning mutually private Scopes. Storage authorization
cannot retract information that is already visible in one model context.

Read tools search and materialize context without mutation. Page, Summary,
Relation, Scope, and validity tools are marked as writes so Codex can apply the
configured approval policy. The server never imports symbiont-d profile,
exploration, or conversation policy.

---

## 技术详情 (Technical Specification)

当前协议 / Current specification:
**[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**

历史版本 / Historical generations:
**[deprecated/](deprecated/README.md)**
