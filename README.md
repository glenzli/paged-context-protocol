# Paged-Context-Protocol (PCP) · v0.8.0-draft

**中文** | [English](README-en.md)

![Paged-Context-Protocol banner](assets/banner.png)

> **协议草案与开发预览。** v0.8 的协议、接口和 Store 格式仍可能调整。v0.8 不兼容 v0.7 Store；迁移需要新建 Store，并从租户保留的原始内容重新导入。

PCP 是一个开放协议及项目维护的 Rust 实现，用于保存、组织和检索可跨任务延续的上下文。它以稳定 Page、不可变 Revision、Scope、Relation 和 Provenance 表达内容，并通过有界查询把需要的部分交给当前任务。

PCP 可以由单个应用在进程内作为上下文层使用，也可以由独立 Runtime 让多个客户端在同一个 Identity 下贡献不同 Scope。具体产品可以把它用于当前任务的上下文管理、长期记忆、项目知识、会话连续性或这些场景的组合。协议不要求独立守护进程、多个租户或后台维护器，也不规定上层如何命名或呈现这些能力。

## 设计范围

```text
租户保留的来源
  -> Identity + Scope                    身份、授权与关联边界
  -> Page + immutable Revision           稳定记录与内容版本
  -> Relation + Provenance + Summary     组织、依据与派生内容
  -> Search + Read + Projection          有界检索与读取
  -> Host / 模型的当前工作上下文
```

- **Page 与 Revision**：Page 是可独立检索的记录；Revision 是不可变内容快照。原始记录通常为 `sealed`，维护型记录可以是 `revisioned`。
- **Identity、Tenant 与 Scope**：一个 Store（无论嵌入 Host 还是由 Runtime 托管）服务一个持久 Identity。租户只能读写获授权的 Scope，请求身份由实现注入。
- **Relation 与 Provenance**：Relation 连接稳定 Page；关系依据和派生来源引用精确 Revision。时间相邻或文本相似不会自动形成领域关系。
- **Search、Read 与 Projection**：检索先返回候选，再按需读取 Payload、Summary、Sources、Relations 或 History。Host 决定哪些结果进入当前上下文。
- **维护与治理**：实现可以提供 Summary、Topic、Validity、Relation、无损 packing 和 retention；项目维护的 Runtime 还提供可选的后台维护与审阅流程。低风险操作是否自动应用由部署配置决定。
- **内容更新与反馈**：租户可以正常写入新信息，也可以针对可读的旧 Revision 提交反馈，分开记录实际使用的上下文和新增纠正证据。维护器提出有效性或替代建议；跨 Scope 的决定和替代、撤回都需在 Console 批准，不会静默改写原 Page。
- **外部来源**：租户保管并理解自己的聊天记录、媒体或领域对象。PCP 保存最小 SourceRef 和可选 digest，并按授权返回来源坐标；来源解析、查询和展示仍由租户负责。

PCP 不规定 Router、提示词格式、Chain-of-Thought、上下文窗口规划或模型状态机。它定义持久记录、授权、来源、检索和可选维护操作的边界。SQLite、独立 Runtime、语义模型、Console 交互和具体 Host 工作流属于实现选择。

## 当前实现

本仓库包含规范和项目维护的 Rust 实现。应用可以通过 embedded client 在进程内组合 Store，也可以通过 `pcp-runtime` 的 RPC 接入同一套对象和租户契约。`pcp-runtime` 是面向本机多客户端部署的参考服务形态，不是协议本身，也不是使用 PCP 的前置条件；Discovery、注册、Observer 和后台调度只属于这种服务形态。仓库另提供 CLI、MCP 和本地 Console。

新客户端可以通过 [Infra Discovery](https://github.com/glenzli/infra-protocol) 发现 Runtime，申请 Principal、访问模式和 Scope，并在用户批准后取得当前 generation 的身份绑定端点。已批准的 registration 可在 Runtime 重启后重新发现并打开新会话。

候选提交后，已启用的后台维护会按同一 Scope 整理相关证据，生成保留时间演变、分歧和来源的记忆草案。一组候选可形成多条记忆，也可对照已有 Page 提议更新或标记已涵盖；在 Console 启用候选自动审核、维护写入模式与共享高级审核额度后，草案稳定约 5 分钟即由 Sol 按输出审核，通过的部分自动写入，未解决的证据继续保留；修订草案需独立核验，Console 可暂停自动审核、查看依据并撤回自动写入。新内容合并等待约 2 分钟，每轮最多整理一个有界窗口，无新证据不重复调用模型。未处理候选持续保留；仅已处理的临时记录到期清理。设计与恢复边界见 [候选演变](design/candidate-evolution.md)。

![PCP Console 使用合成演示数据展示本地 Store 概览](assets/console-overview.png)

*PCP Console 的本地 Store 概览。截图使用合成数据，不包含真实 Page、Scope 或客户端身份。*

### 已实现

- 稳定 Page、不可变 Revision、`sealed`/`revisioned` 行为和 CAS 修订。
- `exact`、`text`、`graph`、`temporal` 与 `auto` 检索，以及有界 Projection 读取。
- Runtime RPC 的 `semantic_search`、`match_intent` 和显式锚点图扩展。
- Summary、Topic、Validity、Relation、Provenance、archive/restore、无损 sealed-Page packing 和访问审计。
- Runtime 注入 Identity 与 Actor 的 `ingest_page`，包括可选 `sourceSpan`、`basedOnRevisionIds` 和最小 SourceRef。
- 租户 `submit_feedback`、逐目标反馈协调、Validity/`supersedes` 原子提交，以及 Luna→Sol→人工的有界升级路径。
- Runtime-local Context Inbox：经 Console 显式启用的候选暂存与短期活动卡；候选经人工审阅或已启用的 Sol 共享额度审核通过后成为正式 Page。
- embedded/RPC client、授权注册、CLI、MCP、Console、维护协调器和只读设施观测。
- 确定性 Revision 保留规划、有限租约和受保护的显式回收。

![PCP Console 使用合成演示数据展示 Page 列表](assets/console-pages.png)

*Pages 视图展示 Page 类型、Scope、来源区间和直接关系。内容均为合成演示数据。*

### 当前边界

- Durable Page deletion、cold storage 和 Identity 全局 Validity 维护尚未实现；`purge` 不属于 v0.8。
- 外部来源的托管、解析、检索、展示、OCR 和转写由租户实现。
- 语义查询依赖显式配置的 Provider；缺少 Provider 时返回不可用，不自动改用关键词查询。
- Context Inbox 不聚合独立 Store，不根据跨客户端重复提及自动确认或晋升候选。
- 本地 Unix socket 的 `0600` 权限是 OS 用户边界，不能防御同一用户下运行的恶意进程。
- 公共协议的合规边界以 [`PROTOCOL.md`](PROTOCOL.md) 为准，而不是本仓库的某个具体后端或界面。

## 仓库结构

| Crate | 职责 |
| --- | --- |
| `pcp-core` | 核心对象、请求、投影与 capability 类型 |
| `pcp-store` | 携带 `AccessSession` 的数据库无关 Store 契约 |
| `pcp-client` | 租户 `PcpTenantApi`、特权 `PcpApi` 与 embedded client |
| `pcp-rpc` | Unix socket 协议、remote client 与 server transport |
| `pcp-sqlite` | SQLite Store、检索、审计与 retention |
| `pcp-runtime` | 身份绑定端点、客户端注册与维护协调器 |
| `pcp-cli` | 检查、检索、读取、导出与保留操作 |
| `pcp-mcp` | 本地 stdio MCP server |
| `pcp-console` | 本地 Store Inspector、审阅和治理入口 |

## 快速开始

Workspace 使用 Rust 2024 edition：

```bash
cargo test --workspace

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- doctor

PCP_STORE_PATH=data/context.sqlite3 \
  cargo run -p pcp-cli -- retention-plan 30 2 100
```

`retention-plan` 是 dry run。实际回收使用 `retention-collect --confirm`，并在提交前重新规划精确 Revision ID。

## 部署

`PcpTenantApi` 是普通租户接口，提供 descriptor、授权 Scope、`ingest_page`、`submit_feedback`、Search、Read 和可选 browse。`PcpApi` 是 Runtime 维护器与本机管理工具使用的特权超集。Host 可以嵌入 Store，也可以连接独立 Runtime：

```text
Tenant Host --> PcpTenantApi --> EmbeddedPcpClient --> PcpStore
                         `-----> RemotePcpClient ----> pcp-runtime --> PcpStore
Codex --------> MCP -----------> PcpTenantApi
Runtime/CLI -------------------> PcpApi
```

启动多客户端 Runtime：

```bash
cargo build --release -p pcp-runtime
target/release/pcp-runtime --config examples/runtime.toml
```

每个 RPC endpoint 绑定一个由 Runtime 注入的 Principal，请求不能自选身份。需要隔离不同租户或模型上下文时，应使用独立端点和最小 Scope。

### macOS 受管服务

PCP 可以通过本地 Console 服务托管 `pcp-runtime`、Store、socket、enrollment state 和 maintenance ledger。默认数据目录为 `~/Library/Application Support/PCP`：

```bash
sh scripts/install-macos.sh
```

LaunchAgent `com.glenzli.pcp-console` 启动 `pcp-console --managed`。生成的 Runtime 配置默认关闭自动维护；部署者配置独立授权的 worker 后，可选择 observe 或 apply 模式。

首次启动前可以导入已有 Store 和 enrollment state：

```bash
sh scripts/import-store.sh \
  --source /absolute/path/to/context.sqlite3 \
  --enrollment-state /absolute/path/to/pcp-enrollments.json
```

Infer Runtime worker 的升级审阅通过 `[maintenance.worker.review_budget]` 配置，默认关闭。启用后，Sol High 使用滚动 24 小时 300 次 / 600 万总 token，Astra Low 默认 10 次。Console 可修改额度、查看实际用量与未结预占；重启不重置账本。旧的无预算升级路径已停用。预算统一采用准入阈值：实际用量与预占控制后续调用，已开始的调用正常完成，允许最后一笔超出 Token 阈值。详见[审阅预算与修正流程](design/maintenance-review-budget.md)。

### MCP

MCP 可以直接打开 embedded Store：

```bash
cargo build --release -p pcp-mcp
codex mcp add pcp \
  --env PCP_STORE_PATH=/absolute/path/to/context.sqlite3 \
  --env PCP_CLIENT_ID=codex:project-example \
  --env PCP_ACCESS_MODE=read \
  --env PCP_ALLOWED_SCOPES=project:example,conversation:example-main \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

也可以连接身份绑定的 Runtime：

```bash
codex mcp add pcp \
  --env PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-codex.sock \
  --env PCP_CLIENT_ID=codex:project-example \
  -- /absolute/path/to/paged-context-protocol/target/release/pcp-mcp
```

长期运行的 MCP 客户端应使用 enrollment，而不是持久保存 generation-specific Runtime socket。`pcp-mcp enroll begin` 会创建 mode `0600` 的本机 credential state 并提交访问申请；Console 批准后运行 `pcp-mcp enroll status` 完成注册。随后在 MCP 配置中传入 `PCP_ENROLLMENT_FILE` 和匹配的 `PCP_CLIENT_ID`，每次启动都会通过当前 Infra Discovery registration 重新打开会话。静态 `PCP_RUNTIME_SOCKET` 仅保留给显式配置的兼容端点。

普通可写租户应使用 `contribute`；它在 Read 基础上增加 `ingest_page` 和针对精确 Revision 的 `submit_feedback`。`repair` 是开发迁移使用的窄管理面：仅在 Read 基础上增加保留历史的 `repair_page`，不授予普通 Page 写入、修订、生命周期或 Scope 管理。应使用独立 Principal/credential，只在显式 apply 迁移期间打开。`write` 和 `admin` 仍仅用于维护器和本机管理工具。完整访问模式与 enrollment 合同见 [`crates/pcp-runtime/ENROLLMENT.md`](crates/pcp-runtime/ENROLLMENT.md)。

### ChatGPT 与 Codex 共用隧道接入

PCP 的日常入口统一为 ChatGPT 中创建的 PCP 连接，Codex 使用同一个连接。链路为：ChatGPT / Codex → OpenAI Secure MCP Tunnel → 本机 `pcp-chatgpt-mcp` → PCP Runtime → Store。隧道主动向 OpenAI 发起 HTTPS 连接，本机不需要开放公网监听；工具请求和返回内容仍会经过 OpenAI。Runtime、Store 和授权文件继续由本机管理。

#### 1. 安装并启动本机 PCP

在本仓库目录执行：

```bash
sh scripts/install-macos.sh
```

安装程序构建并安装 Runtime、Console、`pcp-mcp` 和 `pcp-chatgpt-mcp`。打开 [PCP Console](http://127.0.0.1:4318/)，确认 Runtime 已连接。需要 Rust 构建工具链；隧道客户端另行安装。

#### 2. 创建 PCP 授权

新安装需要用当前 Runtime 的 Infra Discovery registration manifest 发起授权；文件名为 `pcp--idn_....json`，应选择属于目标 Store 的当前注册文件，不能使用旧 socket 或猜测其他实例。manifest 的定位与验证见 [Enrollment 合同](crates/pcp-runtime/ENROLLMENT.md)。

默认 macOS Discovery 目录可按下面列出；若 Runtime 配置了 `INFRA_PROTOCOL_RUNTIME_DIR`，改用该目录下的 `registrations`。多个结果时核对 JSON 中的 `service.instance_id` 与 Console 的 Store identity。

```bash
ls "$(getconf DARWIN_USER_TEMP_DIR)infra-protocol/registrations/"pcp--*.json
```

```bash
pcp_home="$HOME/Library/Application Support/PCP"
"$pcp_home/bin/pcp-chatgpt-mcp" enroll begin \
  "/absolute/path/to/current/pcp--idn_....json"
```

在 Console 的客户端访问中检查并批准 `ChatGPT`，再执行：

```bash
"$pcp_home/bin/pcp-chatgpt-mcp" enroll status
```

授权状态保存在 `clients/chatgpt-pcp.json`。`chatgpt:pcp` 对当前用户 Scope 请求 `contribute`，对其他当前 Scope 请求只读权限。已有可用 ChatGPT 接入时复用此授权和隧道，无需重新 enrollment。Codex 共享该身份；历史命名 `chatgpt_capture` / `captureSurface: chatgpt` 表示这条接入路径，不用于区分实际调用的是哪个客户端。

#### 3. 配置并测试隧道

按 [OpenAI Secure MCP Tunnel 文档](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels) 在 Platform 创建隧道，取得 tunnel ID 和 runtime API key，并安装官方 `tunnel-client`。隧道须关联目标 ChatGPT workspace，操作者须有使用权限；有权创建隧道不等于目标 workspace 已可使用它。

以下命令假定 `tunnel-client` 在 PATH 中；将 `YOUR_TUNNEL_ID` 替换为刚创建的 ID。在本机终端以隐藏输入方式设置密钥，避免把密钥写入仓库或命令历史：

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

隐藏输入语法适用于 macOS 默认 zsh。让前台进程保持运行，在 ChatGPT 开启 Developer Mode，从 Plugins 页面创建连接，名称可设为 `PCP`，Connection 选择 **Tunnel** 并选中同一 tunnel ID。检查发现的工具和写操作权限。界面与可用性受账号、workspace 策略影响，当前入口见 [OpenAI 连接说明](https://developers.openai.com/plugins/deploy/connect-chatgpt)。

#### 4. 设置登录后常驻

前台连接验证成功后，在另一个本机终端执行：

```bash
python3 integrations/chatgpt/install-service.py
```

安装器默认从 `~/Applications/tunnel-client/tunnel-client` 读取客户端、从 `~/.config/tunnel-client/pcp-chatgpt.yaml` 读取 profile；位置不同时用 `--binary /absolute/path/to/tunnel-client` 和 `--profile /absolute/path/to/profile.yaml` 指定。没有环境变量时会隐藏提示输入 runtime key。密钥以 `0600` 权限保存，LaunchAgent 只引用文件路径。

等待安装器报告 ready 且完成 control-plane poll 后，停止旧的前台进程，避免同一隧道长期运行两个 poller。服务登录后自动启动、退出后重启；Mac 睡眠、断网或关机期间不能访问本机 PCP。

```bash
launchctl print "gui/$(id -u)/com.glenzli.pcp-chatgpt-tunnel"
# 安装新版 pcp-mcp 后，让隧道子进程加载新版本：
launchctl kickstart -k "gui/$(id -u)/com.glenzli.pcp-chatgpt-tunnel"
```

本机隧道状态页为 [127.0.0.1:4319/ui](http://127.0.0.1:4319/ui)。profile、密钥、日志位置以及停用/轮换操作见[完整接入说明](integrations/chatgpt/README.md)。

#### 5. 在 ChatGPT 和 Codex 验证同一个入口

在 ChatGPT 新对话和 Codex 新任务中启用同一个 PCP 连接；Codex 不再安装本地 `pcp` 插件。先读取一个已知话题并核对来源，再检查 `pcp_whoami`（默认精简工具集包含此工具）：两边应显示相同 `chatgpt:pcp` Principal、用户 Scope 和授权范围。确认只保留一个日常入口，避免旧任务中残留的本地工具影响判断。

若无法发现工具，依次检查 Console 的 Runtime 状态、ChatGPT enrollment、隧道 ready/poll 状态和 workspace 关联。只通过 `doctor` 不代表服务已运行，也不代表对话已成功调用。变更工具元数据后在连接管理中刷新工具。正式 capture 和 feedback 的确认由宿主操作权限控制，升级或重新连接后仍需检查。

### 维护、Console 与观测

后台维护与 Console 手动运行共用持久审阅队列。Worker 只产生候选，Runtime 和 Store 负责预算、授权、当前 Revision 校验与提交；普通语义 Relation 可单独启用全文独立复核，通过后自动应用；未通过、Archive 和高影响反馈协调建议保留人工审阅。自动维护可覆盖全部授权 Scope，并低频复查旧页；同主题短页的数量或总内容量积累也可触发提炼，不只检查单页长度。Topic 按精确来源修订去重，并参考相邻主题与已记录的拒绝原因；独立全文复核确认新增信息和原有限定后，才可自动写入。无效候选修正一次后隔离，待审 Topic 达到上限时暂停新增提案。达到门槛的 Topic 自动生成需单独启用，跨 Scope 提炼保留全部来源修订与明确的目标 Scope。反馈协调默认由低成本模型判断；只有不确定项才升级一次，更高影响的 `superseded`/`retracted` 仍需人工批准。调度、模型升级和失败退避见 [`crates/pcp-runtime/README.md`](crates/pcp-runtime/README.md)。

Console 应连接独立的 `audit` endpoint。它提供只读 Store 检查、查询预览、注册管理、维护审阅和受权 archive/restore。Runtime 的设施 observer 只返回聚合且脱敏的运行数据；合同见 [`crates/pcp-runtime/OBSERVER.md`](crates/pcp-runtime/OBSERVER.md)。

Console 的访问时间线默认按已完成的 Runtime API 请求计数，使用 Runtime 生成的请求 ID 关联底层操作；展开时才分页读取明细。后台维护有独立来源标记，旧记录保留在未关联操作视图，不按时间猜测请求归属。跨 scope 的请求汇总仅对拥有全部相关 scope 审计权限的调用者可见。语义搜索每次重新枚举有权限访问的当前 revision，同一 embedding space 已缓存的 revision 不再重复读取正文。

```bash
cargo build --release -p pcp-console
PCP_RUNTIME_SOCKET=/absolute/path/to/pcp-operator.sock \
PCP_CLIENT_ID=operator:local \
PCP_CONSOLE_BIND=127.0.0.1:4318 \
  target/release/pcp-console
```

## 为什么不再单独维护 Codex 插件

PCP 统一维护 ChatGPT 连接，并在 Codex 中复用它。两边本来就使用同一份 Store，常驻隧道也已是 ChatGPT 接入的一部分。额外分发 Codex 插件会增加一套安装、版本、授权与使用指引，还会让同一任务看到两组重复工具。统一接入减少这些维护状态与选择歧义；没有必要仅为区分两个客户端而保留第二套入口。

代价是 ChatGPT 与 Codex 都依赖同一条连接器和隧道链路；隧道不可用时两边都会暂时失去 PCP。该链路在正常使用中本就常驻，因此日常不需要新增服务。共享身份仍受原有 Scope 权限约束；客户端名称不作为新增权限的依据。

仓库不再分发独立 Codex 插件、专用 Skill 或 marketplace 快照。`pcp-mcp`、Runtime、Console 与本机隧道启动器继续维护，通用 stdio MCP 仍可用于开发和排障。使用策略通过 MCP instructions、工具描述和本文档维护：按需主动检索、精确读取来源、正式写入保持高门槛。启用近况后，在开始或恢复实质性话题时读取，并在目标、阶段进展、下一步、阻塞、暂停或完成状态变化时更新；无变化不写，不逐消息记录。

从旧插件迁移时，先验证共享连接，再在 Codex 中卸载本地 PCP 插件并新建任务。已有 Store、记忆、ChatGPT enrollment 和常驻隧道无需迁移或重建；旧 Codex enrollment 不再需要时，可单独在 Console 中撤销。这里的私有隧道接入不等于向公开插件目录发布服务。

## 文档

- [面向模型的工具接入与精简返回](integrations/TOOL_INTEGRATION.md)
- [当前协议](PROTOCOL.md)
- [英文协议](PROTOCOL-en.md)
- [Runtime 说明](crates/pcp-runtime/README.md)
- [Enrollment 合同](crates/pcp-runtime/ENROLLMENT.md)
- [Observer 合同](crates/pcp-runtime/OBSERVER.md)
- [历史版本与淘汰原因](deprecated/README.md)
- [MIT License](LICENSE)
