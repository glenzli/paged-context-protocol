# Paged-Context-Protocol (PCP) - v0.7.0-draft

**中文** | [English](README-en.md)

![Paged-Context-Protocol banner](assets/banner.png)

一个开放的上下文分页协议及其官方实现，用于决定用户拥有的信息何时进入模型注意力，以及它以
什么身份、依据和范围进入。

## 定位

尖端模型已经能够搜索文件、调用工具并管理较长的当前上下文，但跨会话、跨项目和跨模型的信息
连续性仍然通常依赖粗粒度压缩或封闭的产品 Memory。对于长期、高密度内容，问题不只是“能否存下”，
而是能否在需要时准确找到、验证来源，并只将当前任务真正需要的部分放入有限注意力。

**PCP 定义一个用户拥有、模型无关的信息空间，以及它与模型注意力之间的分页边界。**它不是某个
特定的 context manager、Memory 产品或 Storage 格式，也不要求把所有历史塞进同一个提示词。
在 PCP 中，Context 是跨时间存在的信息连续体；模型当前窗口只是一个临时 working set。

```text
来源 / 事件
  -> 稳定 Page + 不可变 Revision       （身份与存储）
  -> Relation / Provenance / Summary   （组织与依据）
  -> Search / Read / Projection        （选择与物化）
  -> 当前工作上下文                      （模型注意力）
```

## 协议核心

- **Page 与 Revision**：Page 是稳定语义对象；Revision 是不可变内容快照。原始 Page 通常为
  `sealed`，维护型 Page 可以是 `revisioned`。
- **Scope 与 Access**：统一地址空间不等于全局注入。搜索、读取、派生和写入都受 Scope 与服务端
  注入的访问身份约束。
- **Relation 与 Provenance**：Relation 连接稳定 Page；关系依据和来源链引用精确 Revision。
  时间相邻或文本相似不会自动成为领域关系。
- **Summary 与 Validity**：Summary 是可选、稀疏、可追溯的派生 Page，不是每条内容的强制层级；
  Validity 记录内容当前是否仍适用。
- **Search、Read 与 Projection**：检索先返回可识别候选，再按需读取 Summary、Payload、Sources、
  Relations、History 等投影；模型或 Host 决定查询路径和进入注意力的时机。
- **Consolidation 与 Retention**：多个当前语义 Page 可以经显式整合收缩为 canonical Page；历史
  Revision 只在依赖、租约和保留规则允许时回收。

PCP 不规定固定 Router、Intent Focus、四级变焦、Chain-of-Thought、XML 流程或模型状态机。模型如何
决定写入、搜索、总结、整合和物化，属于 Model Client 或 Host 策略；协议只提供可互操作、可追溯、
受约束的对象与接口。

## 当前状态

`v0.7.0-draft` 是当前协议草案。它在不可变 Revision 之上恢复稳定 Page 身份，并加入显式 Scope、
Provenance、Relation、稀疏 Summary、Validity、consolidation 与 Revision retention 语义。

### 官方实现

本仓库既是 PCP 规范的权威来源，也是由项目维护者发布的官方 Rust 实现。该实现覆盖 Store、embedded
与 remote client、Unix socket RPC、Runtime、自动发现与授权注册、CLI、MCP、Console、维护和观测，
构成可部署的端到端 PCP 系统。

PCP 仍是开放协议，允许独立实现。“官方”表示该实现由 PCP 项目维护和发布，不表示 SQLite、某种
检索算法、某个模型或 Host 工作流自动成为协议要求；合规边界以 [`PROTOCOL.md`](PROTOCOL.md) 为准。

![PCP Console 使用合成演示数据展示本地 Store 概览](assets/console-overview.png)

> 实际 PCP Console 界面；截图使用合成演示数据，不包含真实 Page 内容、Scope 或客户端身份。

| Crate | 职责 |
| --- | --- |
| `pcp-core` | 核心对象、请求、投影与 capability 类型 |
| `pcp-store` | 携带 `AccessSession` 的数据库无关 Store 契约 |
| `pcp-client` | 面向 Host 的传输无关 `PcpApi` 与 embedded client |
| `pcp-rpc` | 本地 Unix socket 协议、remote client 与 server transport |
| `pcp-sqlite` | SQLite Page/Revision Store、迁移、检索、审计与回收 |
| `pcp-runtime` | 身份绑定端点、客户端授权注册与可选维护协调器 |
| `pcp-cli` | 检查、检索、读取、导出、整合与保留操作 |
| `pcp-mcp` | 基于官方 Rust MCP SDK 的本地 stdio 工具服务器 |
| `pcp-console` | 独立、只读的本地 Web Inspector |

### 已实现

- 稳定 Page、不可变 Revision、`sealed`/`revisioned`、CAS 修订与 v0.6 数据幂等迁移。
- head-only 默认检索，`auto`、`exact`、`text`、`graph`、`temporal` 模式，以及有界 Projection 读取。
- Summary、Validity、Relation、Provenance、consolidation 与访问审计。
- 身份绑定的 embedded/RPC client、可发现且经用户批准的 Runtime 注册、CLI、MCP 与 Console。
- 确定性 Revision 保留规划、有限租约、受保护的显式回收，以及多维 Health 诊断。

### 尚未实现

Alias 与 durable Page deletion 当前会通过 Capabilities 报告为不可用，cold storage 也尚未实现。

### 实现边界

官方实现有意不内置语义模型或 Router。部署方可以在协议接口之上选择本地模型、远端模型、全文检索
或组合策略，而不改变 PCP 的对象、权限与可追溯性语义。

## 快速开始

当前 workspace 使用 Rust 2024 edition：

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

## 部署

`PcpApi` 是消费侧边界。同一个 Host 可以直接嵌入 Store，也可以通过可选的 Runtime 使用独立生命周期
和固定服务端身份；CLI 与 MCP 均可连接这两种形态。

```text
Host --------> PcpApi --> EmbeddedPcpClient --> PcpStore
                    `----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex -------> MCP -----> PcpApi
Operator ----> CLI -----> PcpApi
```

多 client 部署可以从 [`examples/runtime.toml`](examples/runtime.toml) 启动 broker；这些静态端点也可在
客户端迁移到自动发现与授权注册期间继续使用：

```bash
cargo build --release -p pcp-runtime
target/release/pcp-runtime --config examples/runtime.toml
```

每个 Unix socket 对应一个由 Runtime 注入的固定 Principal，请求不能自选身份。socket mode 为 `0600`；
这是本地用户边界，不能防御已经以同一 OS 用户运行的恶意进程。需要强隔离时，应使用独立端点、
最小 Scope 和独立模型上下文，因为 Storage 权限无法撤回已经进入模型窗口的信息。

Runtime 还会通过 [Infra Discovery](https://github.com/glenzli/infra-protocol) 发布客户端授权注册能力。
本机客户端可以申请 Principal、访问模式与 Scope，在 Console 中由用户批准后取得当前 generation 的
身份绑定 RPC 端点；Runtime 重启后，客户端重新发现并凭持久 registration 打开新会话，不再依赖
硬编码 socket 路径。

[Symbiont](https://github.com/glenzli/symbiont-d) 迁移顺序与完整合同见
[`crates/pcp-runtime/ENROLLMENT.md`](crates/pcp-runtime/ENROLLMENT.md)。

### Runtime 维护

维护协调器是可选能力，默认只观察，不应用变更。配置的 semantic worker 只能在有界候选与 Detail 上
返回 `write_summary`、`consolidate`、`keep_separate` 或 `defer` 决策，不能直接写 Store；Runtime 补齐
机械元数据，Store 再验证权限、当前 head、lineage 与原子性。维护器不会自动执行 Revision 回收。

详见 [`crates/pcp-runtime/README.md`](crates/pcp-runtime/README.md)。

### 设施观测

Runtime 通过 [Infra Discovery](https://github.com/glenzli/infra-protocol) 发布只读 observer 能力。
[Infra Sentinel](https://github.com/glenzli/infra-sentinel) 经 owner-only Unix socket 读取版本化、聚合且
经过脱敏的 snapshot，包括运行时间、24 小时请求量、失败/拒绝、当前 Page 数，以及可用时的 p95 与
遥测覆盖率。该接口不暴露 Page 内容、查询、Scope 名称、原始审计或维护动作；Console 只是 PCP
snapshot 中可选的深链，不属于 discovery 或 observer 数据接口。

合同与 Python wire 示例见
[`crates/pcp-runtime/OBSERVER.md`](crates/pcp-runtime/OBSERVER.md)。

### Codex / MCP

MCP 可以直接打开 embedded SQLite Store：

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

```bash
codex mcp add pcp \
  --env PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
  --env PCP_CLIENT_ID=codex:project-example \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

`PCP_ACCESS_MODE` 可为 `observe`、`read`、`audit`、`write` 或 `admin`。`observe` 只允许读取聚合
Health，不能列出或读取 Page、搜索、读取原始审计或执行维护动作。即使在 `admin` 模式，跨 Scope 派生
仍需单独启用。`pcp_whoami` 用于检查服务端注入的 Principal 与授权范围；读工具不产生内容写入，
Page、Summary、Relation、Scope 与 Validity 工具会被 MCP 标记为写操作。

### Console

Console 应连接一个独立的 `audit` 端点。其 Store Inspector 只读，提供 Page、Relation、访问时间线、
Retention 和 Health 视图；唯一的控制面动作是批准、拒绝或撤销本机客户端注册。Health 将存储形态、
活动、召回、整合、关系与运行状况分开呈现，不合成为不透明总分；操作遥测不保存查询文本或 Page 内容。

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

## 规范与历史

- 当前协议：[PROTOCOL.md](PROTOCOL.md)
- 英文版协议：[PROTOCOL-en.md](PROTOCOL-en.md)
- Runtime 说明：[crates/pcp-runtime/README.md](crates/pcp-runtime/README.md)
- PCP Runtime observer 合同：[crates/pcp-runtime/OBSERVER.md](crates/pcp-runtime/OBSERVER.md)
- PCP Runtime enrollment 合同：[crates/pcp-runtime/ENROLLMENT.md](crates/pcp-runtime/ENROLLMENT.md)
- 历史版本与淘汰原因：[deprecated/](deprecated/README.md)
- 许可证：[MIT](LICENSE)
