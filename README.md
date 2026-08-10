# Paged-Context-Protocol (PCP) - v0.7.0-draft

![Paged-Context-Protocol banner](assets/banner.png)

A model-agnostic protocol for deciding what user-owned information enters model
attention, when it enters, and with what identity and evidence.

一个模型无关的上下文分页协议，用于决定用户拥有的信息何时进入模型注意力，以及它以什么身份、
依据和范围进入。

## 定位 / Overview

尖端模型已经能够搜索文件、调用工具并管理较长的当前上下文，但跨会话、跨项目和跨模型的信息
连续性仍然通常依赖粗粒度压缩或封闭的产品 Memory。对于长期、高密度内容，问题不只是“能否存下”，
而是能否在需要时准确找到、验证来源，并只将当前任务真正需要的部分放入有限注意力。

Frontier models can search files, call tools, and manage long active contexts,
yet continuity across sessions, projects, and models is still commonly reduced
to coarse summaries or closed product Memory. For long-lived, high-density work,
the problem is not only whether information can be stored, but whether the right
material can be found, traced, and selectively admitted into finite attention.

**PCP 定义一个用户拥有、模型无关的信息空间，以及它与模型注意力之间的分页边界。**它不是某个
特定的 context manager、Memory 产品或 Storage 格式，也不要求把所有历史塞进同一个提示词。
在 PCP 中，Context 是跨时间存在的信息连续体；模型当前窗口只是一个临时 working set。

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

## 协议核心 / Protocol Core

- **Page 与 Revision / Page and Revision**：Page 是稳定语义对象；Revision 是不可变内容快照。
  原始 Page 通常为 `sealed`，维护型 Page 可以是 `revisioned`。/ A Page is a stable semantic
  object; a Revision is an immutable content snapshot. Raw Pages are normally
  `sealed`, while maintained Pages may be `revisioned`.
- **Scope 与 Access / Scope and Access**：统一地址空间不等于全局注入。搜索、读取、派生和写入都受
  Scope 与服务端注入的访问身份约束。/ A unified address space is not global injection.
  Search, read, derivation, and write operations remain constrained by Scope and
  a server-injected access identity.
- **Relation 与 Provenance / Relation and Provenance**：Relation 连接稳定 Page；关系依据和来源链引用
  精确 Revision。时间相邻或文本相似不会自动成为领域关系。/ Relations connect stable Pages,
  while relation evidence and provenance refer to exact Revisions. Temporal
  adjacency or textual similarity does not automatically become a domain edge.
- **Summary 与 Validity / Summary and Validity**：Summary 是可选、稀疏、可追溯的派生 Page，不是每条
  内容的强制层级；Validity 记录内容当前是否仍适用。/ Summaries are optional, sparse,
  traceable derived Pages rather than a mandatory tier for every item. Validity
  records whether information remains applicable.
- **Search、Read 与 Projection / Search, Read, and Projection**：检索先返回可识别候选，再按需读取
  Summary、Payload、Sources、Relations、History 等投影；模型或 Host 决定查询路径和进入注意力的时机。
  / Retrieval returns identifiable candidates before selected projections such
  as Summary, Payload, Sources, Relations, or History are read. The model or Host
  chooses the query path and admission timing.
- **Consolidation 与 Retention / Consolidation and Retention**：多个当前语义 Page 可以经显式整合收缩为
  canonical Page；历史 Revision 只在依赖、租约和保留规则允许时回收。/ Multiple current
  semantic Pages may be explicitly contracted into a canonical Page. Historical
  Revisions are collected only when dependencies, leases, and retention rules permit.

PCP 不规定固定 Router、Intent Focus、四级变焦、Chain-of-Thought、XML 流程或模型状态机。模型如何
决定写入、搜索、总结、整合和物化，属于 Model Client 或 Host 策略；协议只提供可互操作、可追溯、
受约束的对象与接口。

PCP does not prescribe a fixed Router, Intent Focus, zoom hierarchy,
Chain-of-Thought, XML flow, or model state machine. Decisions to write, search,
summarize, consolidate, and materialize remain Model Client or Host policy; the
protocol supplies interoperable, traceable, and constrained objects and interfaces.

## 当前状态 / Current Status

`v0.7.0-draft` 是当前协议草案。它在不可变 Revision 之上恢复稳定 Page 身份，并加入显式 Scope、
Provenance、Relation、稀疏 Summary、Validity、consolidation 与 Revision retention 语义。

`v0.7.0-draft` is the current protocol draft. It restores stable Page identity
above immutable Revisions and defines explicit Scope, Provenance, Relation,
sparse Summary, Validity, consolidation, and Revision-retention semantics.

仓库现在同时包含一个可运行的 Rust 参考实现。它用于验证协议边界，不把 SQLite、某种检索算法、
某个模型或 Host 的操作习惯提升为协议要求。

The repository now includes a working Rust reference implementation. It validates
the protocol boundary without making SQLite, one retrieval algorithm, one model,
or a Host workflow normative.

| Crate | 职责 / Role |
| --- | --- |
| `pcp-core` | 核心对象、请求、投影与 capability 类型 / Core objects, requests, projections, and capability types |
| `pcp-store` | 携带 `AccessSession` 的数据库无关 Store 契约 / Database-independent Store contract with `AccessSession` |
| `pcp-client` | 面向 Host 的传输无关 `PcpApi` 与 embedded client / Transport-independent `PcpApi` and embedded client |
| `pcp-rpc` | 本地 Unix socket 协议、远端 client 与 server transport / Local Unix-socket wire, remote client, and server transport |
| `pcp-sqlite` | SQLite Page/Revision Store、迁移、检索、审计与回收 / SQLite Page/Revision Store, migrations, retrieval, audit, and retention |
| `pcp-runtime` | 身份绑定端点、客户端授权注册与可选维护协调器 / Identity-bound endpoints, approved client enrollment, and optional maintenance coordinator |
| `pcp-cli` | 检查、检索、读取、导出、整合与保留操作 / Inspection, retrieval, read, export, consolidation, and retention operations |
| `pcp-mcp` | 基于官方 Rust MCP SDK 的本地 stdio 工具服务器 / Local stdio tool server built on the official Rust MCP SDK |
| `pcp-console` | 独立、只读的本地 Web Inspector / Independent read-only local Web Inspector |

### 已实现 / Implemented

- 稳定 Page、不可变 Revision、`sealed`/`revisioned`、CAS 修订与 v0.6 数据幂等迁移。
  / Stable Pages, immutable Revisions, `sealed`/`revisioned` behavior, CAS updates,
  and idempotent migration from v0.6 data.
- head-only 默认检索，`auto`、`exact`、`text`、`graph`、`temporal` 模式，以及有界 Projection 读取。
  / Head-only default retrieval, `auto`, `exact`, `text`, `graph`, and `temporal`
  modes, plus bounded Projection reads.
- Summary、Validity、Relation、Provenance、consolidation 与访问审计。
  / Summary, Validity, Relation, Provenance, consolidation, and access audit.
- 身份绑定的 embedded/RPC client、可发现且经用户批准的 Runtime 注册、CLI、MCP 与 Console。
  / Identity-bound embedded and RPC clients, discoverable user-approved Runtime
  enrollment, CLI, MCP, and Console.
- 确定性 Revision 保留规划、有限租约、受保护的显式回收，以及多维 Health 诊断。
  / Deterministic Revision-retention planning, finite leases, protected explicit
  collection, and multidimensional Health diagnostics.

### 尚未实现 / Not Yet Implemented

Alias 与 durable Page deletion 当前会通过 Capabilities 报告为不可用，cold storage 也尚未实现。
参考实现不内置语义模型或 Router；部署方可以在协议接口之上选择本地模型、远端模型、全文检索
或组合策略。

Aliases and durable Page deletion are currently reported as unavailable through
Capabilities, and cold storage is not yet implemented. The reference implementation
does not embed a semantic model or Router; deployments may compose local models,
remote models, full-text retrieval, or other strategies above the protocol interface.

## 快速开始 / Quick Start

当前 workspace 使用 Rust 2024 edition：
The workspace currently uses Rust 2024 edition:

```bash
cargo test --workspace

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- doctor

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- retention-plan 30 2 100
```

`retention-plan` 只是 dry run：三个参数依次表示最小保留天数、每个 Page 至少保留的最近 Revision 数，
以及返回的候选上限。实际回收使用独立的 `retention-collect --confirm`，并在提交前重新规划精确
Revision ID。

`retention-plan` is a dry run. Its arguments are minimum age in days, recent
Revisions retained per Page, and candidate limit. Physical collection uses the
separate `retention-collect --confirm` command and replans exact Revision IDs
before submission.

## 部署 / Deployment

`PcpApi` 是消费侧边界。同一个 Host 可以直接嵌入 Store，也可以通过可选的 Runtime 使用独立生命周期
和固定服务端身份；CLI 与 MCP 均可连接这两种形态。

`PcpApi` is the consumer boundary. A Host may embed a Store directly or use the
optional Runtime for an independent lifecycle and fixed server-side identities.
Both CLI and MCP can connect through either shape.

```text
Host --------> PcpApi --> EmbeddedPcpClient --> PcpStore
                    `----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex -------> MCP -----> PcpApi
Operator ----> CLI -----> PcpApi
```

多 client 部署可以从 [`examples/runtime.toml`](examples/runtime.toml) 启动 broker；这些静态端点也可在
客户端迁移到自动发现与授权注册期间继续使用：
For multiple clients, start the broker from [`examples/runtime.toml`](examples/runtime.toml):

```bash
cargo build --release -p pcp-runtime
target/release/pcp-runtime --config examples/runtime.toml
```

每个 Unix socket 对应一个由 Runtime 注入的固定 Principal，请求不能自选身份。socket mode 为 `0600`；
这是本地用户边界，不能防御已经以同一 OS 用户运行的恶意进程。需要强隔离时，应使用独立端点、
最小 Scope 和独立模型上下文，因为 Storage 权限无法撤回已经进入模型窗口的信息。

Each Unix socket maps to one fixed Principal injected by Runtime; requests cannot
choose their own identity. Socket mode is `0600`. This is a local-user boundary,
not protection from a hostile process already running as the same OS user. Strong
isolation requires separate endpoints, minimal Scopes, and separate model contexts,
because Storage authorization cannot retract information already visible to a model.

Runtime 还会通过 Infra Discovery 发布 `pcp.runtime.enrollment@20260810.1`。本机客户端可以申请
Principal、访问模式与 Scope，在 Console 中由用户批准后取得当前 generation 的身份绑定 RPC 端点；
Runtime 重启后，客户端重新发现并凭持久 registration 打开新会话，不再依赖硬编码 socket 路径。

Runtime also advertises `pcp.runtime.enrollment@20260810.1` through Infra
Discovery. A local client requests a Principal, access mode, and Scopes; after
approval in Console it receives an identity-bound RPC endpoint for the current
generation. Following a Runtime restart, it rediscovers and reopens the durable
registration instead of relying on a hard-coded socket path.

合同与 Symbiont 迁移顺序 / Contract and Symbiont migration:
[`crates/pcp-runtime/ENROLLMENT.md`](crates/pcp-runtime/ENROLLMENT.md).

### Runtime 维护 / Runtime Maintenance

维护协调器是可选能力，默认只观察，不应用变更。配置的 semantic worker 只能在有界候选与 Detail 上
返回 `write_summary`、`consolidate`、`keep_separate` 或 `defer` 决策，不能直接写 Store；Runtime 补齐
机械元数据，Store 再验证权限、当前 head、lineage 与原子性。维护器不会自动执行 Revision 回收。

The maintenance coordinator is optional and defaults to observation without
applying changes. A configured semantic worker may return only `write_summary`,
`consolidate`, `keep_separate`, or `defer` decisions over bounded candidates and
Detail. It cannot write the Store directly; Runtime supplies mechanical metadata,
and the Store revalidates authorization, current heads, lineage, and atomicity.
The maintainer never performs automatic Revision collection.

详见 / See [`crates/pcp-runtime/README.md`](crates/pcp-runtime/README.md).

### 设施观测 / Infrastructure Observation

Runtime 默认通过 `infra.discovery.registration@20260810.1` 发布
`pcp.runtime.observer@20260810.1` offer。Infra Sentinel 经 owner-only Unix socket 读取版本化、
聚合且经过脱敏的 snapshot，包括运行时间、24 小时请求量、失败/拒绝、当前 Page 数，以及可用时的
p95 与遥测覆盖率。该接口不暴露 Page 内容、查询、Scope 名称、原始审计或维护动作；Console 只是
PCP snapshot 中可选的深链，不属于 discovery 或 observer 数据接口。

Runtime advertises `pcp.runtime.observer@20260810.1` through
`infra.discovery.registration@20260810.1`. Infra Sentinel reads a versioned,
aggregate-only, redacted snapshot over an owner-only Unix socket: uptime,
24-hour calls, failures and denials, current Page count, and optional p95 latency
and telemetry coverage. Page content, queries, Scope names, raw audit, and
maintenance actions are excluded. Console is only an optional PCP snapshot deep
link, not part of discovery or the observer data interface.

合同与 Python wire 示例 / Contract and Python wire example:
[`crates/pcp-runtime/OBSERVER.md`](crates/pcp-runtime/OBSERVER.md).

### Codex / MCP

MCP 可以直接打开 embedded SQLite Store：
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

也可以让 MCP 连接已经运行、身份绑定的 Runtime：
Alternatively, MCP can connect to an identity-bound Runtime:

```bash
codex mcp add pcp \
  --env PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
  --env PCP_CLIENT_ID=codex:project-example \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

`PCP_ACCESS_MODE` 可为 `observe`、`read`、`audit`、`write` 或 `admin`。`observe` 只允许读取聚合
Health，不能列出或读取 Page、搜索、读取原始审计或执行维护动作。即使在 `admin` 模式，跨 Scope 派生仍需
单独启用。`pcp_whoami` 用于检查服务端注入的 Principal 与授权范围；读工具不产生内容写入，Page、
Summary、Relation、Scope 与 Validity 工具会被 MCP 标记为写操作。

`PCP_ACCESS_MODE` may be `observe`, `read`, `audit`, `write`, or `admin`. `observe`
can read aggregate Health only; it cannot list or read Pages, search, read raw
audit events, or invoke maintenance actions. Cross-Scope derivation
still requires a separate opt-in even in `admin` mode. `pcp_whoami` reports the
server-injected Principal and grants. Read tools do not mutate content; Page,
Summary, Relation, Scope, and Validity tools are marked as writes for MCP approval.

### Console

Console 应连接一个独立的 `audit` 端点。其 Store Inspector 只读，提供 Page、Relation、访问时间线、
Retention 和 Health 视图；唯一的控制面动作是批准、拒绝或撤销本机客户端注册。Health 将存储形态、
活动、召回、整合、关系与运行状况分开呈现，不合成为不透明总分；操作遥测不保存查询文本或 Page 内容。

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

Console 与 Runtime 默认从各自静态 endpoint 的同级目录使用
`pcp-enrollment-admin.sock`。若 operator endpoint 与 broker 的第一个 endpoint 不在同一目录，需为
二者设置同一个 `PCP_ENROLLMENT_ADMIN_SOCKET` 绝对路径。

Console and Runtime default to `pcp-enrollment-admin.sock` beside their static
endpoint. If the operator endpoint and the broker's first endpoint use different
directories, set the same absolute `PCP_ENROLLMENT_ADMIN_SOCKET` for both.

## 规范与历史 / Specification and History

- 当前协议 / Current specification: **[PROTOCOL.md (中文)](PROTOCOL.md)** · **[PROTOCOL-en.md (English)](PROTOCOL-en.md)**
- Runtime 说明 / Runtime notes: **[crates/pcp-runtime/README.md](crates/pcp-runtime/README.md)**
- PCP Runtime observer contract: **[crates/pcp-runtime/OBSERVER.md](crates/pcp-runtime/OBSERVER.md)**
- PCP Runtime enrollment contract: **[crates/pcp-runtime/ENROLLMENT.md](crates/pcp-runtime/ENROLLMENT.md)**
- 历史版本与淘汰原因 / Historical generations and deprecation rationale: **[deprecated/](deprecated/README.md)**
- License: **[MIT](LICENSE)**
