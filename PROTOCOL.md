# Paged-Context-Protocol (PCP) - v0.4.0-draft

> **状态：草案。** 本文重新定义 PCP Core。它与已归档的
> [v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md) 不向后兼容。

Paged-Context-Protocol（PCP）是一套面向模型的、用户拥有的逻辑上下文基础协议。这里的
Context 不仅指模型当前窗口，也包括跨会话、跨项目、跨时间存在于 Storage、Memory 与
active attention 中的连续信息空间。PCP 使不同模型能够通过普通工具接口访问和维护这一
空间，而不要求模型遵循固定的路由、压缩、变焦或推理流程。

PCP 的核心定位是：

> **定义跨持久 Storage、模型 Memory 与 active attention 的统一 Page 空间，以及内容在
> 其中被发现、投影和物化的语义；不定义模型应当如何推理。**

## 0. 规范语言与范围

本文中的“必须”、“不得”、“应当”和“可以”分别对应规范性要求中的 MUST、MUST NOT、
SHOULD 和 MAY。

PCP Core 定义：

- Page、Revision、Scope、Provenance 和 Relation 的语义；
- 可选 Summary Projection 与 Detail 读取的寻址和来源约束；
- 总结、派生与聚合关系所形成的可回溯无环子图；
- 有界候选结果、读取结果与 Context Bundle 进入模型注意力时的物化边界；
- Page 的写入、读取、搜索、修订、关联和生命周期操作；
- 原始历史与模型派生内容之间的可恢复边界；
- 不同模型和不同存储实现之间的互操作约束。

PCP Core 不定义：

- 模型或 Host 使用何种策略决定何时搜索、读取、写入或纳入注意力；
- 模型使用 grep、全文检索、语义检索还是图遍历；
- 当前上下文窗口如何压缩、折叠或排序；
- 固定的 Router、Worker、Consolidator 或 Auditor 拓扑；
- 固定 Prompt、XML 模板、Chain-of-Thought 或变焦状态机。

## I. 设计原则

### 1.1 模型拥有当前上下文

当前模型或其 Harness 负责管理 active working set。PCP 不干预模型在有效上下文窗口内的
阅读顺序、推理方式、摘要策略或工具调用习惯。

### 1.2 用户拥有长期上下文

长期上下文不应绑定于单一模型、单一会话、单一项目目录或单一服务商。一个符合 PCP 的
Store 应允许经授权的不同模型访问同一逻辑地址空间。

### 1.3 Page 身份独立于驻留位置

Page 是否正在某个模型窗口中、是否被缓存、是否仅存在于持久存储中，不改变其逻辑身份。
当前上下文只是 Page 的一次临时投影。

### 1.4 原始历史负责可恢复性，派生 Page 负责可用性

在用户授权与保留策略允许时，Host 应保存可搜索的原始会话、工具事件和来源记录。模型可以
在其上创建摘要、结论、关系和其他派生 Page，但派生内容不得破坏原始来源。

### 1.5 约束语义，不约束策略

协议应严格约束身份、版本、作用域、来源和写入行为，同时允许模型自由选择搜索与上下文
组织策略。模型能力越强，协议操作手册应越薄，而持久化边界不应因此变弱。

### 1.6 注意力物化必须可识别

候选元数据、Summary、payload 或 source span 一旦被 Host 暴露给 Model Client，就已经以
相应分辨率进入模型可见注意力。PCP 不决定哪些内容此刻应被选中，但必须区分 Store 内部
候选、已返回的有界发现结果和已读取的 Detail，并保留它们对应的 Page、Revision 与
Projection 身份。

## II. 系统角色与边界

PCP 只定义逻辑角色，不规定部署拓扑。

### 2.1 Model Client

调用 PCP 接口的模型或 Agent。它可以搜索、读取、写入、修订和关联 Page，并自行决定调用
顺序。多个 Model Client 可以使用完全不同的工具习惯。

### 2.2 Host

将聊天、项目、工具和模型运行时连接到 PCP Store 的应用。Host 负责：

- 身份认证、授权和 Scope 选择；
- 在允许时捕获原始事件；
- 执行 Token、延迟和结果数量预算；
- 将 PCP 接口暴露为 MCP、function tools、CLI、HTTP 或本地函数；
- 将选定的 Page Projection 物化到模型当前上下文，并按 Continuity Host 要求记录该边界。

### 2.3 PCP Store

持久化 Page、Revision、Relation、原始事件和索引的系统。实现可以使用文件、SQLite、关系
数据库、搜索引擎、对象存储或混合后端。

### 2.4 Adapter

把外部文件、仓库、消息、数据库或其他内容与知识系统映射到 PCP Page 的组件。Adapter 必须
保留可回溯的来源信息，不得把一次模型摘要伪装成原始来源。

## III. 逻辑地址空间与 Scope

### 3.1 统一不等于全局注入

PCP 使用统一逻辑地址空间，但所有 Page 不会自动出现在所有任务中。统一寻址只表示 Page
可以用一致方式访问；可见性与召回范围由 Scope 和授权决定。

### 3.2 Scope

每个 Page Revision 必须声明：

- `owner_id`：长期上下文的所有者；
- `namespace`：Page 的主要命名空间，例如用户、项目、任务、会话或分支；
- `visibility`：实现定义的可见性或 ACL 引用。

一个 Page 可以通过 Relation 连接其他 Scope，但这种连接不得自动授予读取权限。

推荐的命名空间形式：

```text
user:<user-id>
project:<project-id>
task:<task-id>
conversation:<conversation-id>
branch:<branch-id>
```

### 3.3 召回范围

Search 请求必须显式指定允许访问的 Scope 集合，或引用由 Host 解析的 Scope Policy。实现
不得因为语义相似而静默扩大到未授权项目。

Host 可以提供以下策略，但它们不是 Core 的固定枚举：

- `strict`：仅当前 Scope；
- `linked`：当前 Scope 加显式关联的 Scope；
- `global`：用户授权的全局范围。

### 3.4 接入端身份与访问会话

PCP 必须区分内容的 `Actor` 与访问 Store 的 `AccessPrincipal`：

- `Actor` 表示谁产生、修订或总结了内容，并进入 Provenance；
- `AccessPrincipal` 表示哪个 Host、模型客户端、CLI 或服务正在访问 Store；
- `AccessSession` 由受信任的接入层建立，绑定 Principal、会话 ID 和精确的 Scope Grant；
- `ScopeGrant` 声明该 Principal 在一个 Scope 中可执行的操作，例如 Search、读取 Summary、
  读取 Detail、Write、Revise、Summarize、Link、Assess 或管理 Scope。

模型不得通过普通工具参数自行声明或扩大 AccessSession。MCP stdio 实现可以将一项服务器配置
视为一个 Principal；远程传输则必须由认证层把凭据映射为 AccessSession。

Store 必须把 Grant 作为强制上限，并在相关性排序、图遍历和 Projection 物化之前执行授权。
Relation 不授予另一端的读取权限；无权读取另一端时，关系投影和图搜索不得泄露其 Revision。

将一个 Scope 的内容派生、总结或判断到另一个 Scope，可能构成信息降级。实现必须默认拒绝，
除非 AccessSession 对目标 Scope 拥有显式的跨 Scope 派生权限。普通的跨 Scope Relation 不等于
内容派生，也不得自动获得该权限。

Grant 不能撤销已经进入同一模型上下文的信息。需要强信息流隔离时，Host 不得让同一
AccessSession 同时拥有敏感来源 Scope 的读取权和另一目标 Scope 的写入权；应使用不同接入
实例、不同模型上下文或等价的受信任信息流控制。`derive_across_scopes` 约束标准 PCP 派生
操作，但不能把一个已经同时获得读写能力的模型变成安全的去分类器。

兼容实现应记录不含查询正文和 Page 内容的访问元数据，包括 Principal、Session、操作、Scope、
时间与结果。访问日志本身仍受 Scope 和 Audit 权限约束。

## IV. Page 与 Revision

### 4.1 Page

Page 是可以被模型独立寻址和组合的最小持久逻辑对象。Page 不等于固定 Token 块，也不要求
存在摘要。

Page 可以表示：

- 一段原始对话或工具事件；
- 文件、代码、图片或其他来源的引用；
- 用户提出的想法、问题或约束；
- 模型生成的笔记、结论或摘要；
- 一组 Page 的逻辑组合；
- 项目状态、决策、反例、被否定路线或开放问题。

### 4.2 稳定身份与不可变 Revision

- `page_id` 标识跨时间稳定的逻辑对象。
- `revision_id` 标识该对象的一次不可变版本。
- 修订 Page 必须创建新的 `revision_id`。
- 读取只提供 `page_id` 时，Store 应返回调用者可见的最新有效 Revision，并明确实际返回的
  `revision_id`。
- 写入操作应支持 `expected_revision_id`，用于检测并发更新冲突。

`page_id` 不得仅依赖容易碰撞的短哈希。实现可以使用 UUID、ULID、完整内容哈希或其他具备
足够冲突安全性的标识方案。

### 4.3 最小 Page Envelope

每个 Page Revision 必须包含：

```json
{
  "page_id": "pg_01...",
  "revision_id": "rev_01...",
  "owner_id": "user_01...",
  "namespace": "project:formal-math",
  "visibility": "private",
  "lifecycle_status": "active",
  "created_at": "2026-07-15T20:00:00+08:00",
  "created_by": {
    "actor_type": "user|model|tool|system",
    "actor_id": "..."
  },
  "payload": {
    "media_type": "text/markdown",
    "content": "..."
  },
  "source_refs": [],
  "provenance": [
    {
      "operation": "write",
      "actor_type": "user",
      "actor_id": "...",
      "timestamp": "2026-07-15T20:00:00+08:00",
      "input_revision_ids": []
    }
  ]
}
```

`payload` 与 `source_refs` 至少必须有一个非空。Store 可以把大 payload 存放在外部对象存储
中，并在 Page 中保留稳定引用。`provenance` 必须至少包含本 Revision 的创建、摄入或导入
事件。

### 4.4 自由 Payload 与可选 Facets

协议不固定 payload 的领域结构。实现应声明媒体类型或 schema 标识，例如：

```json
{
  "payload": {
    "media_type": "text/markdown",
    "content": "..."
  },
  "facets": {
    "keywords": ["compactness", "finite product"],
    "anchors": ["Theorem 4.7", "Definition 2.1"],
    "symbols": ["X", "K_i"]
  }
}
```

`facets` 全部可选。Facet 是索引或读取辅助，不得被解释为原始证据的替代品。

### 4.5 Summary Projection 与 Detail 读取

`summary` 是由模型、规则或用户为目标 Revision 生成的可选路由 Projection。Detail 则是
从目标 Revision 读取 `payload`、`source_spans` 或其他高分辨率证据的统称，不要求存在一个
名为 `detail` 的持久 Projection。Summary 的主要用途是让模型在有限 Token 预算内判断是否
继续读取 Detail；它不是原始证据，也不得成为唯一可恢复副本。

不是每个 Page 都必须拥有 Summary。Host 可以根据内容长度、密度或模型判断安排摘要任务，
但协议不规定固定阈值，也不要求模型为低价值或短内容生成摘要。没有 Summary 的 Page 仍可
通过 payload 全文、精确字符串、时间、来源和 Relation 被召回。

Summary 在逻辑上必须表示为一个独立 Derived Page Revision，通过 `summarizes` Relation
绑定到精确的目标 `revision_id`，并至少保留：

- Summary 内容；
- 创建者与创建时间；
- 生成它的模型、工具或规则（若适用）；
- 指向目标 Revision 和其他输入 Revision 的 provenance。

实现可以在物理上内联、sidecar 存储或单独持久化 Summary，但逻辑接口必须暴露其独立
`page_id`、`revision_id`、`summarizes` Relation 和 provenance。Summary 可以与目标
Revision 在同一事务中创建，但不得作为目标 Revision 的可变 facet。Read 接口必须把可用
Summary 作为目标 Revision 的可发现 `summary` Projection 暴露。生成或更新 Summary 不得
原地修改目标 Revision 或既有 Summary Revision。

Summary 与 Detail 是可按需选择的读取分辨率，不是强制运行时状态。PCP 不规定模型必须先
读 Summary，也不规定固定的 `Summary -> Detail -> Unpacked` 调度流程。

### 4.6 Source Page 与 Derived Page

PCP 不要求固定 Page 类型枚举，但实现必须能够区分：

- **Source-backed**：直接包含原始事件或可稳定拉回的来源；
- **Derived**：由模型、规则或后台任务从其他 Page 推导、整理或压缩而来。

Derived Page 必须通过 `provenance` 或 `derived_from` Relation 指向输入 Revision。Derived Page
不得静默覆盖 Source-backed Page。

## V. Provenance 与 Relation

### 5.1 Provenance

Provenance 记录内容如何进入 Store。每个事件至少应包含：

```json
{
  "operation": "ingest|write|revise|derive|import",
  "actor_type": "user|model|tool|system",
  "actor_id": "...",
  "timestamp": "...",
  "input_revision_ids": [],
  "tool_or_model": "optional identifier"
}
```

Provenance 证明来源链，不证明内容为真。事实可靠性、指令权限和数据完整性不得被压缩进一个
单一 `trust` 枚举。

### 5.2 Relation

Relation 是带类型、可寻址来源和创建者的有向边：

```json
{
  "relation_id": "rel_01...",
  "from_revision_id": "rev_a",
  "type": "depends_on",
  "to_revision_id": "rev_b",
  "created_by": {"actor_type": "model", "actor_id": "..."},
  "created_at": "..."
}
```

Core 推荐但不封闭以下类型：

- `contains`
- `aggregates`
- `derived_from`
- `summarizes`
- `depends_on`
- `defines`
- `uses`
- `supports`
- `contradicts`
- `supersedes`
- `inspired_by`
- `related_to`

领域 Adapter 可以增加新的 Relation 类型。Store 必须保留类型原文，不得把所有关系折叠为
无类型相似度边。

普通 Relation 图可以包含环，例如互相 `related_to` 的节点。由 provenance 输入和
`derived_from`、`summarizes`、`aggregates` 形成的整理子图必须保持无环。`contains` 保留为
一般成员关系，不自动具有无环语义；派生聚合层级必须使用 `aggregates`。Store 必须拒绝会在
整理子图中引入直接或传递环的写入。这样模型可以从 Summary 或聚合 Page 逐层回到精确来源，
而不会进入无限整理环。

## VI. 原始事件与模型记忆

本节中的“模型记忆”指由 Summary、Derived Page 和 Relation 构成的逻辑组织层，不表示 PCP
依赖一个独立 Memory 服务、数据库或 Profile。相同 PCP Store 可以同时承载原始来源、派生
组织和供 active context 读取的 Projection。

### 6.1 原始事件流

为避免模型忘记写入或过早判断信息不重要，Continuity Host 应在授权范围内将以下内容作为
Source-backed Page 或可搜索 Event 保存：

- 用户与模型消息；
- 工具调用及必要结果；
- 项目、任务和分支标识；
- 文件或外部状态变化；
- 用户显式要求保留的内容。

原始事件默认不等于 active context，也不要求每轮召回。

### 6.2 模型维护的派生层

模型可以使用普通 Write、Revise 和 Link 接口建立更高质量的长期 Page，例如：

- 当前项目状态；
- 重要决定和理由；
- 跨项目灵感；
- 已验证结论；
- 失败路线与负面结果；
- 对一组历史事件的摘要或索引。

后台维护模型与前台工作模型使用同一套接口。PCP 不定义独立 Consolidator 处理器。

### 6.3 稀疏 Summary Index 与渐进整理

实现可以只为模型认为值得建立语义入口的 Revision 生成 Summary。由这些 Summary 构成的
索引是对原始 Page 空间的稀疏派生视图，而不是完整历史的替代品。

模型或后台任务可以进一步把一组相关 Source-backed 或 Derived Page 整理成新的聚合
Derived Page。聚合 Page 必须通过 provenance 与 `aggregates` 指向精确输入 Revision；其
内容若由输入推导，还应使用 `derived_from`，它自身也可以拥有可选 Summary。多层聚合形成
可逐层下钻的 DAG，而不是必须覆盖全部 Page 的单一树。

是否生成单页 Summary、是否建立聚合 Page、从 Summary 下钻 Detail 还是绕过索引直接搜索
payload，均由 Model Client 或 Host 策略决定。协议只保证这些路径可以组合、追溯和重建。

### 6.4 非破坏性整理

摘要、合并、去重和重组必须产生新 Revision、Relation 或 Derived Page。它们不得删除唯一的
原始来源。物理删除只由用户授权和 Host 保留策略决定。

## VII. Core 接口

本节定义逻辑语义，不规定传输。字段可按实现扩展，但不得改变核心行为。

### 7.1 DescribeCapabilities

让 Model Client 发现 Store 能力。结果应包括：

- 支持的搜索模式；
- 支持的 Projection，包括是否支持独立写入 Summary；
- 支持的 Scope 类型、Policy 与发现能力；
- 分页、结果大小和 payload 限制；
- 支持的 Relation 与 schema 扩展；
- 是否支持事件摄入、版本冲突检测和 durable deletion。

模型不应依赖未声明的能力。

### 7.2 ListScopes

分页列出或搜索调用者可访问的 Scope，使模型能够发现当前项目之外的历史与项目空间，而
不必预先知道 namespace。结果应包含：

- Scope 标识、可读名称与可选描述；
- Scope 类型及其父级或显式链接；
- 调用者权限；
- 最近活动时间与可选 Page 数量统计；
- 分页 cursor。

ListScopes 只暴露授权元数据，不应因为列出 Scope 就读取其中的 Page 内容。

### 7.3 SearchPages

在授权 Scope 中返回候选 Page Revision。

```json
{
  "query": "此前是否讨论过有限乘积保持紧致性的证明路线？",
  "scopes": ["project:formal-math", "user:user_01"],
  "mode": "auto",
  "projections": ["summary", "payload"],
  "filters": {
    "relation_types": ["depends_on", "inspired_by"],
    "created_before": null,
    "lifecycle_status": ["active", "superseded"]
  },
  "limit": 20,
  "cursor": null
}
```

搜索模式可以包括：

- `exact`：精确字符串、符号或 ID；
- `text`：grep、正则、全文检索或 BM25；
- `semantic`：语义候选召回；
- `graph`：Relation 遍历；
- `temporal`：时间和版本查询；
- `hybrid`：多通道组合；
- `auto`：由 Store 或 Model Client Adapter 选择。

待搜索的 Projection 与匹配方法是正交维度。`summary`、`payload` 和显式 facets 属于搜索
表面，不是搜索模式；例如调用者可以对 `summary` 执行 `text` 或 `semantic` 搜索。实现可以
允许空 query 配合 `temporal` 或 `auto`，有界浏览近期 Summary Index。

PCP 不规定最终相关性算法。语义分数、全文分数和图距离不得被假装为天然可比较的统一真值。

Search 结果至少应返回：

- `page_id` 与 `revision_id`；
- Scope；
- 匹配片段或可用 facets；
- 实际命中的 Projection（例如 `summary` 或 `payload`）、匹配通道与简短 match metadata；
- 可继续读取的 Projection；
- 分页 cursor。

Search 是有界发现与路由接口，不得隐式物化目标 Detail。返回给模型的匹配片段本身已经是
低分辨率 attention materialization，因此必须声明命中的 Revision、Projection 和范围，并
计入结果预算。实现不得通过 `contentParts`、trace 或其他元数据字段夹带完整 Detail。若命中
`summary` Projection，除有界匹配片段外，完整 payload 应只通过后续显式 Read 获取。

### 7.4 ReadPages

读取已知 Page。推荐的 Projection：

- `manifest`：Envelope 与可用能力；
- `summary`：与目标 Revision 绑定的可选路由摘要及其 provenance；
- `payload`：当前 Revision 的内容；
- `source_spans`：指定来源范围；
- `relations`：入边、出边或指定类型关系；
- `facets`：keywords、anchors、symbols 等可选索引；
- `history`：Revision 历史。

Projection 是一次读取请求的视图，不是目标 Page 的驻留状态。Summary 与 Detail 可以形成
自然的按需读取路径，但 PCP 不定义固定的 `Summary -> Detail -> Unpacked` 状态机。

### 7.5 WritePage

创建新 Page。请求应支持：

- payload 或 source reference；
- Scope；
- 可选 facets；
- 可选初始 Relations；
- provenance；
- idempotency key。

Store 必须返回最终 `page_id` 和 `revision_id`。

### 7.6 RevisePage

为既有 `page_id` 创建新 Revision。请求应包含 `expected_revision_id` 或显式声明允许基于旧
Revision 分叉。Store 不得原地覆盖已发布 Revision。

### 7.7 LinkPages

在 Revision 之间创建 typed Relation。实现可以允许 Relation 自身拥有 Revision，但至少必须
保留创建者、时间和来源端点。

### 7.8 WriteSummary

为一个精确目标 Revision 创建或修订可选 Summary Projection。请求应支持：

- `target_revision_id`；
- 修订既有 Summary 时的 `summary_page_id`；
- Summary 内容；
- 创建者、模型或工具标识；
- 指向目标 Revision 和其他输入 Revision 的 provenance；
- 可选 `expected_summary_revision_id`；
- idempotency key。

Store 必须返回 Summary Derived Page 的 `summary_page_id` 与 `summary_revision_id`。修订
既有 Summary 时，调用者未提供正确的 `expected_summary_revision_id`，Store 应报告冲突，
而不是静默覆盖。创建另一个 Summary 不得静默替换目标已有 Summary。Summary 的写入权限
不得高于读取其目标和输入 Revision 的权限。

实现可以采用不同物理布局，但必须维持 Derived Page、`summarizes` Relation 与独立 Revision
的逻辑语义，不得原地修改已经发布的目标 Revision。

### 7.9 SuppressPages

在指定任务、会话或查询范围内降低或禁止 Page 召回。Suppress 不改变 Page 的 durable
状态，不应自动传播为全局负反馈。

### 7.10 TombstonePage 与 DeletePage

- `TombstonePage` 表示 Page 已废弃、撤回或不应进入普通召回，但保留审计和关系完整性。
- `DeletePage` 表示物理删除，必须受用户权限、保留策略和法规要求控制。
- Model Client 可以建议 tombstone，不应默认拥有不可恢复删除用户历史的权限。

### 7.11 IngestEvent

Host-facing 接口，用于写入原始消息、工具事件或项目状态变化。Ingest 必须支持幂等键，以避免
重试产生重复历史。

## VIII. 搜索与模型自治

### 8.1 不规定搜索计划

模型可以：

- 先列出 Scope，再执行 grep；
- 先搜索符号，再遍历 `depends_on`；
- 先做语义召回，再逐页读取原文；
- 直接按时间查询某次对话；
- 并行执行多种检索并自行比较结果。

这些行为都不属于协议状态机。

### 8.2 结构约束与语义发现可以并存

领域系统可以把显式依赖作为强制候选，同时让模型发现未建边的潜在关系。例如数学系统可以
先计算定理依赖闭包，再让模型补充类比、历史讨论和隐藏前提。PCP 不强制哪一种结果拥有最终
优先级，但接口必须保留结果来源与关系类型。

### 8.3 Attention Materialization 与 Context Bundle

任何 Search 匹配片段或 Read Projection 被 Host 暴露给 Model Client 时，都发生了一次
attention materialization。仍停留在 Store 内部的候选不算已进入模型注意力。PCP 不规定
Host 或模型采用何种准入算法，但 Continuity Host 应能够记录：

- `context_id`、任务或会话标识；
- `page_id`、`revision_id`、Projection 与可选 source span；
- 物化时间与可选 Token 估算；
- 可选选择原因、查询或上游候选引用。

模型或 Host 可以把多个 Page Projection 组合成一次上下文输入。Context Bundle 可以是临时
响应，也可以被写成 Derived Page，但它不是 PCP Core 的固定运行时容器。

Bundle 应保留每个片段的 `page_id`、`revision_id` 和来源，不应把多个 Page 无标记地拼接成
不可追溯文本。

## IX. 生命周期与时间

### 9.1 多时间维度

实现应区分：

- `created_at`：该 Revision 写入 Store 的时间；
- `observed_at`：来源事件发生或被观察的时间；
- `valid_from` / `valid_to`：内容在现实或项目状态中的有效时间，若适用。

不得仅凭存储时间判断事实的新旧。

### 9.2 更新与冲突

新信息可能：

- 修订同一 Page；
- `supersedes` 旧 Revision；
- `contradicts` 另一 Page；
- 只在不同 Scope 或条件下同时成立。

Store 应保留这些差异，不得仅因文本相似而把冲突内容自动去重。

### 9.3 Lifecycle Status

Core 定义以下 `lifecycle_status`：

- `active`：参与普通召回；
- `superseded`：已被新内容替代，但仍可显式读取；
- `archived`：默认不参与普通搜索，但仍可按显式条件读取；
- `tombstoned`：已撤回或废弃，仅按审计或恢复策略可见。

Lifecycle 状态变化必须创建新 Revision 或独立、可审计的 lifecycle event，不得原地修改
不可变 Revision。Archive、Suppress、Tombstone 和 Delete 是不同操作。

## X. 安全、权限与认识论元数据

### 10.1 权限先于相关性

Store 必须在检索和读取前执行访问控制。未授权 Page 即使高度相关也不得进入候选结果。

### 10.2 跨 Scope 不得静默发生

跨项目或用户级召回必须由请求范围或显式关系允许。返回结果必须携带原始 Scope，使模型和
用户能够识别跨项目内容。

### 10.3 数据不自动成为指令

外部来源、历史对话和模型派生 Page 默认应作为数据处理。Host 负责定义哪些来源可以提供
运行时指令，并通过工具权限和执行边界限制最坏影响。

### 10.4 不使用单轴 trust 代替多个问题

实现可以分别记录：

- `authority`：谁有权给出指令或修改状态；
- `integrity`：内容是否原始、派生、签名或校验；
- `epistemic_status`：asserted、corroborated、contradicted、superseded 等；
- `sensitivity`：数据敏感级别；
- `instruction_policy`：data-only 或允许作为指令来源。

这些字段描述不同维度，不得因为内容“已审计”就自动推断其事实为真。

### 10.5 用户控制

用户必须能够查看、导出、限制和删除其长期上下文。模型生成的隐藏摘要不得成为无法审计的
唯一长期副本。

## XI. 传输与访问表面

PCP 是语义协议，不是传输协议。兼容实现可以提供：

- MCP tools；
- JSON-RPC 或 HTTP API；
- 本地函数库；
- CLI；
- 文件系统视图；
- SQLite 或其他查询接口。

一个 Store 可以同时暴露多种访问表面。例如偏好 shell 的模型可以使用 grep/CLI，偏好
structured tools 的模型可以使用 JSON API；它们必须指向同一 Page 与 Revision 身份。

## XII. 最小一致性要求

一个 **PCP Core Store** 至少必须：

1. 持久化稳定 `page_id` 与不可变 `revision_id`；
2. 强制 Scope 与访问控制；
3. 支持 Describe、ListScopes、Search、Read、Write、Revise 和 WriteSummary 的逻辑语义；
4. 保留 provenance 与 source references；
5. 区分持久 Page 与单次读取 Projection；
6. 保留调用者写入的 Summary、其目标 Revision 和派生来源，并作为可发现 Projection 返回；
7. 阻止总结与派生子图中的直接或传递环；
8. 不以摘要替换唯一原始来源；
9. 支持分页或结果预算，避免一次返回无界上下文。

一个 **PCP Continuity Host** 还应：

1. 在授权范围内摄入会话与项目事件；
2. 把当前模型身份、Scope 和权限传递给 Store；
3. 为模型暴露能力发现与基本 Page 操作；
4. 保留检索结果的 Page/Revision 引用；
5. 区分内部候选与已暴露给模型的 Projection，并记录可用的 attention materialization 引用；
6. 将不可恢复删除留给用户或明确的保留策略。

一个 **PCP Adapter** 至少必须：

1. 为外部内容提供稳定 `source_ref`；
2. 标明导入和派生过程；
3. 不把外部结果的临时排序分数当作 Page 身份；
4. 不丢失调用者有权保留的来源定位信息。

## XIII. 非目标

PCP 不试图：

- 替代模型提供商的上下文窗口或原生 compaction；
- 替模型或 Host 决定具体内容何时应进入注意力；
- 保证模型一定会在最佳时机搜索或读取；
- 规定唯一的检索、索引或排序算法；
- 把所有历史自动注入每次请求；
- 通过存储时间或模型总结自动确定事实真伪；
- 替代应用层权限、隐私和执行安全系统。

## XIV. 待决问题

- Page payload 的通用 schema 是否应保持完全开放，或提供少量标准 profile；
- Relation 是否需要独立 Revision 与权限模型；
- Attention Materialization 与 Context Bundle 是否需要标准化的预算、排序和保留元数据；
- 如何表达多模型对同一 Derived Page 的分歧；
- 如何在用户可控删除与来源图完整性之间取得一致行为；
- 哪些事件应由 Continuity Host 默认摄入，哪些必须显式授权；
- 如何建立面向百万级高密度上下文的跨模型一致性评测。
