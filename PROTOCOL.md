# Paged-Context-Protocol (PCP) - v0.7.0-draft

> 状态：草案。v0.7 恢复稳定 Page 与不可变 Revision 的双层模型，并明确区分语义、版本与物理保留策略。

Paged-Context-Protocol（PCP）是一套面向模型、由用户拥有的长期上下文协议。它让不同 Host
和模型在明确 Scope 与权限下发现、读取、写入并追溯同一组持久信息，但不规定固定的召回、
总结、压缩或推理流程。

PCP 的边界是：

> **维护稳定 Page、不可变 Revision、Page 关系与精确来源；把何时召回、如何理解、是否写入
> 和是否进入当前注意力留给模型与 Host。**

## 1. Core 对象

### 1.1 Page

Page 是稳定的语义对象，不等于一次内容版本：

```json
{
  "pageId": "pg_01...",
  "headRevisionId": "rev_03...",
  "ownerId": "usr_01...",
  "namespace": "project:example",
  "kind": "document",
  "mutability": "sealed|revisioned",
  "lifecycleStatus": "active|superseded|archived|tombstoned",
  "createdAt": "...",
  "updatedAt": "..."
}
```

- `sealed`：原始消息、文件快照、工具结果等事实记录。产生后不得发布第二个 Revision。
- `revisioned`：Summary、Topic、用户画像、项目状态等系统持续维护的理解。
- `mutability` 是内容不变量，不是历史保留级别。
- `lifecycleStatus` 描述 Page 是否参与默认召回，不表达其内容真假。
- `kind` 是开放字符串，用于路由和 Host 策略；它不产生新的 Core 对象类型。

### 1.2 Revision

Revision 是 Page 的不可变内容快照：

```json
{
  "revisionId": "rev_03...",
  "pageId": "pg_01...",
  "previousRevisionId": "rev_02...",
  "createdAt": "...",
  "createdBy": { "actorType": "model", "actorId": "..." },
  "payload": { "mediaType": "text/markdown", "content": "..." },
  "sourceRefs": [],
  "facets": {},
  "provenance": []
}
```

已经存在的 Revision 不得原地改写。`revisioned` Page 通过比较并交换
`expectedRevisionId -> newRevisionId` 原子推进当前头。普通版本推进由
`previousRevisionId` 表达，不得创建同一 Page 内的 `supersedes` Relation。

Revision 不可变不等于永久保留。Store 可以按策略回收不再受保护的历史 Revision；Page
当前头、sealed Page 的证据 Revision、仍受保护的 provenance 所依赖的 Revision，以及显式
保留根不得被回收。默认 Search 和全文索引只覆盖 Page 当前头。

### 1.3 Relation

Relation 是稳定 Page 之间可维护的语义断言：

```json
{
  "fromPageId": "pg_summary",
  "relationType": "summarizes",
  "toPageId": "pg_source",
  "basisRevisionIds": ["rev_summary_3", "rev_source_7"]
}
```

`basisRevisionIds` 记录断言建立时观察的精确版本；导航跟随稳定 Page，审计回到精确 Revision。
Core 约定关系包括：

- `summarizes`：来源 Page 是目标 Page 的路由摘要；
- `assesses`：来源 Page 判断目标 Page 当前应如何使用；
- `supersedes`：一个 Page 在语义上替代另一个 Page；
- `aggregates`：来源 Page 聚合多个独立 Page。

`derived_from` 的精确信息应优先写入 Revision provenance；只有确有导航价值时才同时建立
Page Relation。Relation 不能因为时间相邻、同属 Scope、共同命中或向量相似自动产生。
会话顺序属于 Host event stream，不是 Page 图关系。

同一三元组的 Relation 应被去重。错误 Relation 可以撤回；撤回不修改两端 Page。Relation
不会授予另一端 Page 的读取权限。

### 1.4 Provenance

Provenance 属于 Revision，必须引用实际参与生成的精确 `revisionId`。模型或工具生成内容时应
记录操作、Actor、时间、输入 Revision 和必要的工具/模型标识。Page Relation 用于导航，
provenance 用于复现“当时依据了什么”，二者不可互相替代。

### 1.5 Scope 与 Alias

每个 Page 必须属于一个 `namespace`。Search、Read、图遍历和写入均受 AccessSession 的
Scope Grant 限制；语义相似不得扩大权限。

Alias 是可选的人类入口或兼容重定向：`alias -> pageId`。Alias 不是 Page 身份、证据或派生图
节点。旧版 Ref 可以迁移为 Alias，但不得继续承担稳定 Page 的语义。

## 2. Summary、Validity 与 Consolidation

Summary 是普通的 `revisioned` Page，通过 `summarizes` 指向目标 Page。并非每个 Page 都值得
Summary；只有内容足够长、密集或未来值得路由时才创建。更好的 Summary 更新同一个 Summary
Page，产生新 Revision，而不是制造新 Page。

典型召回路径是：

```text
Search/Browse current Summary and Page heads
  -> model selects stable Page IDs
  -> Read current or exact Revision
  -> optionally follow Page Relations or exact provenance
```

Validity assessment 同样是稳定的 `revisioned` Page；新的判断更新同一 Page，而不是累积新的
assessment Page。Page 的 `lifecycleStatus` 只控制默认可见性；`live`、
`qualified`、`disputed`、`retracted` 等认识必须由 assessment 内容、证据 Revision 与当前投影
表达，不能混入版本链。

Consolidation 用于多个 Page 实际表达同一持久对象时的有损收敛：

1. 选择一个 `revisioned` canonical Page；
2. 读取并锁定所有输入 Page 的精确当前 Revision；
3. 为 canonical Page 发布一个新 Revision；
4. provenance 记录全部输入 Revision；
5. canonical Page 以 `supersedes` 指向被吸收的其他 Page；
6. 被吸收 Page 退出默认召回，但仍可精确审计。

canonical Page 与被吸收 Page 必须具有相同的 `kind` 和 `mutability`。原始证据、维护型理解、
Summary 与 Topic 即使文本高度相似，也不能跨语义角色 consolidation；它们应通过 provenance、
`summarizes` 或其他 Page Relation 保持联系。

Host 若为 Page 声明了稳定领域身份（例如一个 episode/topic 的稳定键），身份冲突必须视为
consolidation 的硬拒绝，而不能被标题、文本相似度或共同召回覆盖。该身份的具体字段由 Host 定义，
不进入 PCP 通用协议。

相似度、共同召回和时间邻接只能发现候选。是否合并以及如何有损生成内容必须由 Host、用户或
模型判断，Store 只验证权限、并发、图不变量和事务原子性。

## 3. 接口语义

Core 能力面至少应提供：

- `search_pages(query, scopes, strategy?, limit?, cursor?)`
- `read_pages(page_ids, revision_ids?, view?, max_chars?)`
- `write_page(kind, mutability, content, scope?, based_on_revision_ids?)`
- `revise_page(page_id, expected_revision_id, content, based_on_revision_ids?)`
- `consolidate_pages(canonical_page_id, expected_canonical_revision_id,
  absorbed[{page_id, expected_revision_id}], content)`
- `write_summary(target_page_id, target_revision_id, content)`
- `assess_validity(target_page_id, target_revision_id, standing, evidence_revision_ids)`
- `relate_pages(from_page_id, relation_type, to_page_id, basis_revision_ids?)`
- `plan_revision_retention(scopes, policy)`
- `collect_revision_retention(scopes, policy, confirmed_revision_ids)`
- `put_revision_retention_lease(revision_id, reason, expires_at, idempotency_key)`
- `list_active_revision_retention_leases(scopes, limit)`

默认模型工具应简短。Host 自动填充身份、时间、Scope、常规 provenance 和结构性关系。Search
返回候选而不是真值，并必须有界、可分页、标明命中投影和当前 Revision。精确 Revision 读取
用于审计，不应重新进入默认搜索结果。

## 4. 分层责任

协议定义：Page/Revision 身份、sealed/revisioned 不变量、Page Relation、精确 provenance、
Scope 权限、CAS 发布与可追溯回收约束。

Store/Runtime 负责：当前头索引、事务、权限执行、审计、Relation 撤回、历史保留、冷热迁移、
GC 根、候选发现和维护任务生命周期。这些物理状态不进入模型上下文。

Host 负责：原始事件捕获、会话顺序、Page kind、哪些对象可修订、Summary/Topic/画像策略、语义
关系判断、模型路由、主动探索、注意力边界和当前上下文组装。

PCP 不定义固定 Prompt、向量算法、总结阈值、后台 Agent 拓扑或用户画像格式。

## 5. 保留与回收

实现可以把 Revision 归为当前、受保护、可回收、冷存或缺页占位，但这是 Store 状态，不是
Page/Revision 的协议字段。最小安全规则：

- 当前头永远保留；
- sealed Page 的唯一证据 Revision 默认保留；
- 受保护根可达的 provenance、Relation 的精确端点与 basis、Summary/Validity 精确记录、显式快照
  或租约引用的 Revision 保留；
- 未被引用的普通中间 Revision 可按策略压缩或删除；
- 删除前必须重新计算保护根，并留下聚合级审计记录；
- 被回收 Revision 的 ID 不得静默复用。

精确读取已回收 Revision 时，Store 必须明确返回不可用，而不能静默退回 Page 当前头。
`previousRevisionId` 表示发布顺序，不是永久保留边；回收后版本链可以存在物理缺口，读取端不得沿
缺口猜测或替换版本。

执行回收前，Runtime 应先提供确定性的 dry-run 规划。规划至少返回扫描与保护数量、候选 Revision
和 Page 数量、按原因聚合的保护根、候选内容估算字节数，以及有界的候选与受保护样本。估算字节
只用于比较候选规模，不承诺数据库文件会立即释放相同空间。

规划器从当前头、sealed 证据、最近版本窗口、最小年龄窗口、Relation 的精确端点与 basis、
Summary/Validity 精确记录、当前投影、未过期幂等记录和显式租约等根出发，再沿受保护 Revision
的跨 Page provenance 传递保护。候选 Revision 自身的 provenance 不应反向保活整个待回收子图；
同一 Page 的普通 `previousRevisionId` 也不是 GC 根。跨 Scope 依赖必须保守保护授权范围内的输入，
但不能泄露未授权对象。

显式保留应使用有期限、可幂等续期的 Revision lease，而不是把保留状态写进 Page 内容。lease
必须绑定精确 Revision、授权 Scope、持有 Principal、理由和到期时间；过期 lease 不再构成保护根。
Runtime 可以把真实回收候选的有界路由内容交给 Host 的语义 worker 判断，但模型只选择候选和说明
理由，不选择全局 GC 参数，也不能绕过 Store 的权限与保护闭包。永久保留、提前撤销和实际回收属于
显式运维动作，不应由一次普通模型判断静默触发。

dry-run 不产生删除。执行计划时必须要求具有独立 collection 权限的调用方提交精确候选 ID，
在同一事务内重新计算保护根；任何 ID 已不再是候选时，整批操作必须拒绝。成功回收应原子清理
候选 Revision、只属于候选的兼容索引或来源边，以及与这些候选精确关联且已经超过窗口的幂等记录，
并保存不含正文的 collection ledger，使旧 Revision ID 可被识别为“已回收”而不是“从未存在”。
仍可能被 Host 重放的 Page 写入、当前头和其他存活操作，其幂等记录不能仅因时间经过而全局删除。
实现应分别声明
`supportsRevisionRetentionPlanning` 与 `supportsRevisionRetention`；支持规划不表示已经支持执行。

保留策略按 Host、Page kind、存储预算和价值配置，不应要求模型为每次写入选择 GC 参数。

## 6. v0.6 迁移

v0.6 参考实现虽然公开使用 `Ref ≈ Page`、`Page ≈ Revision`，数据库已保存稳定 `page_id` 和
精确 `revision_id`。升级到 v0.7 时应：

1. 恢复旧稳定 `page_id` 为 Page；旧 `revision_id` 为 Revision；
2. 由 Page 当前头回填 owner、Scope、kind、mutability 与 lifecycle；
3. 由同 Page 的时间序回填 `previousRevisionId`；
4. 删除同 Page 内机械生成的 `supersedes`；
5. 将 Relation 端点归一为稳定 Page，并保留精确 basis Revision；
6. 把 Summary 与 Validity 更新分别收敛到稳定维护 Page；
7. 重建只包含当前头的全文与 Summary 索引；
8. 移除身份型旧 Ref；实现若确实提供 Alias API，再显式迁移非身份型 Ref。

迁移必须事务化、幂等，并在升级前保留可恢复备份。
