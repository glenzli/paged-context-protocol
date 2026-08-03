# Paged-Context-Protocol (PCP) - v0.6.0-draft

> 状态：草案。v0.6 收回了早期 Page/Revision 双层对象与独立 Summary、Validity
> 版本系统。Core 现在以不可变 Page、Page 间 Relation 和可选 Ref 为中心。

Paged-Context-Protocol（PCP）是一套面向模型、由用户拥有的长期上下文协议。它让不同
Host 和模型在明确 Scope 与权限下，发现、读取、写入并追溯同一组持久信息，而不规定模型
必须采用哪一种召回、总结或推理流程。

PCP 的边界是：

> **维护可寻址、不可变、可追溯的 Page 图；把何时召回、如何理解、是否写入和是否进入
> 当前注意力留给模型与 Host。**

## 1. Core 对象

### 1.1 Page

Page 是 PCP 中唯一的持久内容对象。一个 Page：

- 拥有全局稳定的 `page_id`；
- 属于一个 `namespace`（Scope）；
- 创建后内容不可原地修改；
- 可以保存文本 payload、外部大对象引用或二者之一；
- 可以通过 Relation 引用其他 Page；
- 可以是原始事件，也可以是模型产生的总结、判断或聚合。

最小 Page envelope：

```json
{
  "pageId": "pg_01...",
  "ownerId": "usr_01...",
  "namespace": "project:example",
  "visibility": "private",
  "lifecycleStatus": "active",
  "createdAt": "2026-08-03T10:00:00+08:00",
  "createdBy": {
    "actorType": "user|model|tool|system",
    "actorId": "..."
  },
  "payload": {
    "mediaType": "text/markdown",
    "content": "..."
  },
  "sourceRefs": [],
  "provenance": []
}
```

`payload` 与 `sourceRefs` 至少应有一个非空。大图片、音频和其他大型资源可以由对象存储
保存，Page 只记录稳定引用、媒体类型、摘要性描述与来源。

`facets` 可以作为实现定义的可选索引提示，但不是 Core 身份，不得替代 payload、来源或
Relation，也不应无条件进入模型上下文。

### 1.2 Relation

Relation 是两个不可变 Page 之间的有向事实：

```json
{
  "fromPageId": "pg_new",
  "relationType": "supersedes",
  "toPageId": "pg_old",
  "createdAt": "...",
  "createdBy": { "actorType": "model", "actorId": "..." }
}
```

Core 关系：

- `supersedes`：前者成为后者的新解释、状态或内容后继；后者仍可读取；
- `summarizes`：前者是后者的路由摘要；
- `derived_from`：前者直接使用了后者的信息；
- `assesses`：前者判断后者当前应如何被使用。

Host 可以增加 `responds_to`、`supports`、`contradicts`、`aggregates` 等领域关系。派生关系
`summarizes`、`derived_from` 和 `aggregates` 必须形成 DAG。`supersedes` 必须形成可追溯的
有向后继链，不得产生环。

Relation 必须来自明确断言，而不是 Store 根据两个 Page 的时间相邻、写入顺序、同属一个
Scope、搜索命中或向量相似自动生成。特别是：

- `responds_to` 表示可证明的回复或生成因果，不等于“这是用户在上一条助手消息之后说的”；
- `continues` 表示内容上的延续，需要 Host、用户或模型作出语义判断；
- 单纯的会话顺序应由时间投影或 Host 的 event stream 维护，不应以 `follows` 污染 Page 图。

Store 只应自动创建当前操作能够确定的结构性关系，例如 `supersedes`、`summarizes`、
`assesses`，以及调用方明确提供输入 Page 时的 `derived_from`。领域语义关系由 Host、用户或
模型产生。

Relation 是独立于 Page 内容的可维护断言。撤回错误、过期或机械生成的 Relation 不会修改
两端 Page；实现应记录撤回 Actor、时间与原因。精确历史审计可以保留被撤回 Relation，但
默认 Search 和图遍历不得继续使用它。

Relation 不会授予另一端 Page 的读取权限。

### 1.3 Ref

Ref 是可选的、可变的定位名称：

```text
ref_id -> head_page_id
```

Ref 用于“当前用户画像”“当前项目状态”这类需要稳定入口的对象。更新 Ref 不会改变旧 Page，
而是创建新 Page、建立 `supersedes`，再原子地推进 Ref。

Ref 不是内容对象、不是证据，也不参与派生 DAG。模型产生的引用与来源必须最终落到精确
`page_id`，不能只记录一个未来会移动的 Ref。

### 1.4 Scope

每个 Page 必须属于一个 `namespace`。推荐形式：

```text
user:<user-id>
project:<project-id>
task:<task-id>
conversation:<conversation-id>
```

统一地址空间不等于全局注入。Search、Read、图遍历和写入都必须受 AccessSession 的 Scope
Grant 限制；语义相似不得静默扩大权限边界。

## 2. Summary 与 Detail

Summary 不是 Page 的字段版本，也不是第二套存储对象。它是普通的派生 Page，通过
`summarizes` 指向目标 Page。

只有内容足够长、密集或未来值得路由时，才应创建 Summary。短消息和低价值事件可以只参与
精确、关键词或时间检索，不要求每个 Page 都有 Summary。

典型召回路径是：

```text
Search/Browse compact routing text
  -> model selects candidate Page IDs
  -> Read exact Page content
  -> optionally follow Relations or provenance
```

模型也可以直接精确搜索、全文搜索、图遍历或读取已知 Page。PCP 不规定固定的
summary-detail 状态机。

更好的 Summary 必须创建新 Summary Page，并以 `supersedes` 指向旧 Summary。多 Page 的主题
整理同样是普通聚合或 Summary Page，由关系连接来源。

### 2.1 多 Page 合并

长期运行的 Memory 层不能只增加 Summary 与 Relation。模型判断多个当前 Page 实际表达同一
持久主题、事实或状态时，可以创建一个内容自洽的新 Page，并以该 Page `supersedes` 所有
被替代 Page。这个操作称为 consolidation，而不是 Summary：新 Page 是后续召回可直接使用的
内容，不只是通往旧 Detail 的索引。

Consolidation 必须是原子的，并且：

- 输入至少包含两个仍为当前状态的精确 Page IDs；
- 明确选择一个 canonical Page 作为合并结果的身份；所有输入 Ref 原子地收敛到新 Page，调用方
  获得 canonical Ref；
- 新 Page 的 provenance 记录全部输入和 `consolidate` 操作；
- 所有输入仍可精确读取和沿 lineage 追溯，但退出默认 Search、索引和当前关系图；
- 输入不一致、相互矛盾或只是部分相关时不得强行合并，应保留、聚合或 assessment。

相似度、时间邻接和共享 Scope 只能帮助发现候选，不能自行触发 consolidation。语义判断和
有损内容生成必须由 Host、用户或模型完成。实现可以提供可选后台维护器，但 PCP 不规定固定
调度周期、相似度阈值或模型。

## 3. 有效性与变化

Page 的内容永远不被后续事实改写。后续信息可以：

- 以新 Page `supersedes` 旧 Page；
- 创建 assessment Page，以 `assesses` 指向目标，并以 `derived_from` 指向证据；
- 使用 `supports`、`contradicts` 或实现定义关系补充语义。

当同一目标有多个 assessment 时，新 assessment 应 `supersedes` 旧 assessment。Store 可以
投影当前 standing，例如 `live`、`qualified`、`disputed`、`superseded` 或 `retracted`，但
这个投影必须能回到产生判断的 assessment Page 与证据 Page。

读取默认可以返回“当前有效 Page”，但必须允许按精确 `page_id` 读取旧 Page 和完整 lineage。

## 4. Provenance

模型或工具生成的 Page 应记录：

- 创建 Actor 与时间；
- 直接输入的精确 Page IDs；
- 产生内容的操作或工具/模型标识；
- 必要的外部 source reference。

Host 应自动填充可确定的身份、时间、Scope 与 provenance，避免要求模型重复生成机械元数据。
模型只需提供内容、意图和它实际使用的 Page IDs。

Summary、聚合与 assessment 不能伪装成原始证据。任何派生链都应能回到仍受权限与保留策略
约束的来源。

## 5. 模型接口

符合 Core 的能力面应至少提供：

- `search_pages(query, scopes, strategy?, limit?, cursor?)`
- `read_pages(page_ids, view?, max_chars?)`
- `write_page(content, scope?, based_on_page_ids?)`
- `supersede_page(target_page_id, content, based_on_page_ids?)`
- `consolidate_pages(canonical_page_id, replaced_page_ids, content)`
- `write_summary(target_page_id, content, based_on_page_ids?)`
- `assess_validity(target_page_id, standing, rationale, evidence_page_ids)`
- `relate_pages(from_page_id, relation_type, to_page_id)`

Host 可以提供 Scope 浏览、Ref 解析、审计和管理接口。对模型公开的默认工具应保持简短：
结构性关系、Actor、时间、幂等键和常规 provenance 应由 Host 自动产生。

Search 返回的是候选，不是真值。实现可以使用全文索引、精确匹配、时间顺序、图遍历、向量
检索或混合检索，但必须：

- 返回有界结果与游标；
- 标明命中的 Page ID、Scope、投影和截断文本；
- 不越过 AccessSession；
- 允许模型随后读取精确 Page；
- 默认避免把已被 `supersedes` 的 Page 当作当前候选，同时保留精确读取能力。

## 6. Host 与 Store 职责

Host 负责：认证、AccessSession、Scope 选择、预算、原始事件捕获、event stream 顺序、领域
语义关系判断、工具编排，以及哪些内容进入当前模型上下文。

Store 负责：不可变 Page、调用方断言的 Relation、结构性 Relation、Ref 的原子推进、原子
consolidation、当前有效 Page 投影、索引、权限执行、持久化、审计和完整性检查。Store 不根据
时间邻接或相似度发明领域语义关系，也不自行决定哪些内容应被有损合并。

PCP 不定义：用户画像策略、主动探索策略、注意力价值判断、固定 Prompt、模型路由、窗口压缩
算法或后台 Agent 拓扑。这些属于 symbiont-d 等具体 Host。

## 7. 兼容与迁移

从 v0.4/v0.5 的 Page/Revision 实现迁移到 v0.6 时：

1. 每个旧 `revision_id` 成为一个不可变 `page_id`；
2. 旧 `page_id` 成为可选 Ref；
3. 同一旧逻辑对象的相邻 Revision 回填为 `supersedes` 链；
4. Summary sidecar 物化为 Summary Page，并建立 `summarizes` 与必要的 `supersedes`；
5. Validity assessment 物化为 assessment Page，并建立 `assesses`、`derived_from` 与必要的
   `supersedes`；
6. 迁移必须幂等，并在升级前保留可恢复备份。

参考实现可以在内部暂时保留旧表名与 Rust 类型名，但公开 JSON、模型工具与 Console 应使用
v0.6 的 Page、Relation、Ref 语义。
