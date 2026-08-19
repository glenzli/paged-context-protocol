# Paged-Context-Protocol (PCP) - v0.8.0-draft

**中文** | [English](README-en.md)

![Paged-Context-Protocol banner](assets/banner.png)

一个开放的上下文分页协议及其官方实现，用于决定用户拥有的信息何时进入模型注意力，以及它以
什么身份、依据和范围进入。

## 定位

尖端模型已经能够搜索文件、调用工具并管理较长的当前上下文，但跨会话、跨项目和跨模型的信息
连续性仍然通常依赖粗粒度压缩或封闭的产品 Memory。对于长期、高密度内容，问题不只是“能否存下”，
而是能否在需要时准确找到、验证来源，并只将当前任务真正需要的部分放入有限注意力。

**PCP 定义一个用户拥有的 Identity，以及这个持续信息空间与模型注意力之间的分页边界。**它不是某个
特定的 context manager、Memory 产品或 Storage 格式，也不要求把所有历史塞进同一个提示词。
在 PCP 中，Context 是跨时间存在的信息连续体；模型当前窗口只是一个临时 working set。

```text
多租户来源 / 事件
  -> Identity + Scope                    （关联边界与授权）
  -> 稳定 Page + 不可变 Revision       （身份与存储）
  -> Relation / Provenance / Summary   （组织与依据）
  -> Search / Read / Projection        （选择与物化）
  -> 当前工作上下文                      （模型注意力）
```

## 协议核心

- **Page 与 Revision**：Page 是值得独立召回的最小语义片段；Revision 是不可变内容快照。原始 Page
  通常为 `sealed`，维护型 Page 可以是 `revisioned`。
- **Identity、Tenant 与 Scope**：一个 Store/Runtime 服务一个持久 Identity；多个租户可以贡献并共享
  关联空间，但只能读取各自获授权的 Scope。
- **Scope 与 Access**：统一地址空间不等于全局注入。搜索、读取、派生和写入都受 Scope 与服务端
  注入的访问身份约束。
- **Relation 与 Provenance**：Relation 连接稳定 Page；关系依据和来源链引用精确 Revision。
  时间相邻或文本相似不会自动成为领域关系。
- **Summary 与 Validity**：Summary 是可选、稀疏、可追溯的派生 Page，不是每条内容的强制层级；
  Validity 记录内容当前是否仍适用。
- **Search、Read 与 Projection**：检索先返回可识别候选，再按需读取 Summary、Payload、Sources、
  Relations、History 等投影；模型或 Host 决定查询路径和进入注意力的时机。
- **Pack 与 Retention**：来源连续、尚未被引用的细粒度 sealed Page 可以无损 pack 为一个 Page；历史
  Revision 只在依赖、租约和保留规则允许时回收。有损凝炼原始内容不属于 v0.8。
- **外部媒体**：图片、音频与视频可由租户保管，PCP 保存最小、可校验的 SourceRef 及其
  可检索语义表示；原件不可用时必须显式降级，不能静默丢失上下文。

PCP 不规定固定 Router、Intent Focus、四级变焦、Chain-of-Thought、XML 流程或模型状态机。消费模型
决定当前任务查询、读取和物化什么；Runtime 负责 Identity 范围内的长期 Summary、Validity、Relation、
无损 pack 与 retention 维护，并可调用可替换的模型作为语义判断 Provider。

## 当前状态

`v0.8.0-draft` 是当前协议草案。它明确区分 Identity、租户 Principal 和 Scope，把全局维护权归入
Runtime，并以最小 SourceRef 与简化 ingest 接口接收文本和外部媒体来源。v0.8 不兼容 v0.7 Store；
正式迁移将从租户保留的原始内容重新导入。

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
| `pcp-client` | 面向租户的 `PcpTenantApi`、Runtime 维护用 `PcpApi` 与 embedded client |
| `pcp-rpc` | 本地 Unix socket 协议、remote client 与 server transport |
| `pcp-sqlite` | SQLite Page/Revision Store、检索、审计与回收 |
| `pcp-runtime` | Identity 绑定端点、客户端授权注册与全局维护协调器 |
| `pcp-cli` | 检查、检索、读取、导出与保留操作 |
| `pcp-mcp` | 基于官方 Rust MCP SDK 的本地 stdio 工具服务器 |
| `pcp-console` | 独立、只读的本地 Web Inspector |

### 已实现

- 稳定 Page、不可变 Revision、`sealed`/`revisioned` 与 CAS 修订。
- head-only 默认检索，`auto`、`exact`、`text`、`graph`、`temporal` 模式，以及有界 Projection 读取。
- Runtime RPC 的 `semantic_search` 与分预算 `match_intent` Context 查询；结果以结构化 Page/Revision
  条目返回，由调用方决定提示词组装，不内置固定 Context Pack 前缀。
- 以稳定 `pageId` 为锚点、深度/节点/边数受限且 ACL 逐跳过滤的图切片；不提供全库图导出。
- Summary、Validity、Relation、Provenance、无损 sealed-Page packing 与访问审计。
- allowed 访问事件以最多 512 条或 1 秒的有界批次写入，自动提交至少间隔 500 ms；队列过载时
  反压而不静默丢弃。denied/failed 进入 writer 后使用最多 100 ms 的安全合并窗口，并在调用返回前
  持久化。原始 allowed 日志保留 30 天、每批最多清理 5,000 条；安全相关事件不会被该策略自动清理。
- 身份绑定的 embedded/RPC client、可发现且经用户批准的 Runtime 注册、CLI、MCP 与 Console。
- 由 Runtime 注入 Identity/Actor 的简化 sealed `ingest_page`、支持连续来源区间的 `sourceSpan`，以及
  仅包含 provider、locator、可选 media type 和 digest 的 SourceRef。
- 确定性 Revision 保留规划、有限租约、受保护的显式回收，以及多维 Health 诊断。

### 尚未实现

Durable Page deletion 当前不会出现在 Capabilities 的 `features` 中；cold storage、媒体字节托管、
外部 Provider 解析、自动 OCR/转写，以及 Identity 全局 Validity 维护任务也尚未实现。

### 实现边界

官方实现不把固定语义模型或 Router 写入 Store 契约。Runtime 拥有语义检索/意图匹配的 Provider、预算、
校验和提交权；未配置对应 Provider 时查询会明确不可用，绝不静默降级为关键词搜索。Console 只是同一 RPC
查询协议的调试和审阅客户端，不保存或私有持有 Provider 凭据。

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

`PcpTenantApi` 是普通租户边界，只提供 descriptor、授权 Scope、`ingest_page`、Search、Read 与可选
browse。`PcpApi` 是 Runtime maintainer 和本机管理工具使用的特权超集，包含高级写入、Relation、Summary、
Validity、pack、retention 与审计。Host 可以嵌入 Store，也可以通过 Runtime 使用独立生命周期和服务端注入
身份；两种部署形态不改变租户接口。

```text
Tenant Host --> PcpTenantApi --> EmbeddedPcpClient --> PcpStore
                         `-----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex --------> MCP -----------> PcpTenantApi
Runtime/CLI -------------------> PcpApi
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

### PCP 自托管本机服务

在 macOS 上，PCP 通过自己的受管 Console 服务托管 `pcp-runtime`。Console 拥有 Runtime 子进程、
Runtime 配置、Store、socket、enrollment state、maintenance ledger 及 Console 深链；默认根目录是
`~/Library/Application Support/PCP`。租户只能发现并注册，不拥有 Runtime 的启动、重启或配置权。

```bash
sh scripts/install-macos.sh
```

安装后的 LaunchAgent 为 `com.glenzli.pcp-console`，执行 `pcp-console --managed`。Console 只对自己
启动的 Runtime 显示重启控制，并在稳定 operator socket 就绪后才报告成功。生成的
`config/runtime.toml` 默认关闭 maintenance；显式配置 worker 后，worker 即使属于某个租户，cadence 与
ledger state 仍由 PCP Runtime 维护。

首次启动前可通过 PCP 的一致性 SQLite backup 导入已有 Store；同时传入 enrollment state 可保留已经批准的
注册：

```bash
sh scripts/import-store.sh \
  --source /absolute/path/to/context.sqlite3 \
  --enrollment-state /absolute/path/to/pcp-enrollments.json
```

### Runtime 维护

维护协调器是可选能力，默认只观察，不应用变更。配置的 semantic worker 只能返回 Summary 内容、
有序 pack 候选、两个 Page 的 `related_to` 候选、retention milestone、`no_candidate` 或 `defer`，不能直接
写 Store。Runtime 控制候选、预算、关系类型、basis Revision 和提交，Store 再验证权限、精确当前
Revision、来源连续性、外部引用与事务原子性。pack 与 Relation 维护默认关闭，必须单独启用；维护器也
不会自动执行 Revision 回收。官方 Runtime 可使用独立授权的
[Infer Runtime](https://github.com/glenzli/infer-runtime) consumer，也保留本地 command worker。

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

`PCP_ACCESS_MODE` 可为 `observe`、`read`、`contribute`、`audit`、`write` 或 `admin`。普通可写租户应使用
`contribute`：它在 Read 基础上只增加 `ingest_page`，不会授予 revise、Summary、Relation、Validity 或
pack。`write` 与 `admin` 是 Runtime 维护器和本机管理工具的特权模式。`observe` 只允许读取聚合 Health，
不能列出或读取 Page、搜索、读取原始审计或执行维护动作。跨 Scope 派生始终需要单独启用。
`pcp_whoami` 用于检查服务端注入的 Principal 与授权范围。
连接 Runtime 时，MCP 还提供 `pcp_semantic_search`（默认语义搜索）、`pcp_match_intent`（Router 意图匹配）与
`pcp_expand_graph`（显式锚点的有界图切片）。embedded Store 模式不会伪造这些 Runtime Provider 能力。

### Console

Console 应连接一个独立的 `audit` 端点。其 Store Inspector 只读，提供 Page、Relation、访问时间线、
Retention 和 Health 视图；控制面动作为批准、拒绝或撤销本机客户端注册，以及在 Console 自己托管 Runtime
时重启该 Runtime。Health 将存储形态、
活动、召回、pack、关系与运行状况分开呈现，不合成为不透明总分；操作遥测不保存查询文本或 Page 内容。
查询页通过 Runtime RPC 展示结构化返回值，并将预览作为 Console 的本地展示，而不是另一套检索实现。

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
