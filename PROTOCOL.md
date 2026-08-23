# Paged-Context-Protocol (PCP) - v0.8.0-draft

> 状态：草案。v0.8 在 Page/Revision 模型之上明确 Identity、租户与 Runtime 维护权，并收敛来源与关系语义。

Paged-Context-Protocol（PCP）是一套面向模型、由用户拥有的长期上下文协议。它让不同 Host
和模型在明确 Scope 与权限下发现、读取、写入并追溯同一组持久信息，但不规定固定的召回、
总结、压缩或推理流程。

PCP 的边界是：

> **在一个用户拥有的 Identity 中接收多租户输入，维护稳定 Page、不可变 Revision、来源与跨 Scope
> 关系，并向模型提供当前任务最相关、可追溯且经过权限裁剪的有效上下文。**

## 1. Core 对象

### 1.1 Identity、Principal 与 Scope

Identity 是 PCP 的持久上下文与维护边界。同一 Identity 可以接收任意数量租户的输入；Runtime 可以在
Identity 内发现跨租户候选并维护关系，但不得跨 Identity 推断、连接或召回内容。官方 Runtime 当前将
一个 Store 绑定到一个 Identity。

租户通过服务端注入的 Principal 与 AccessSession 操作 PCP。Principal 表示调用者，不是内容所有者。
Scope 是 Identity 内的授权切片。任何读取、搜索、图遍历和写入都必须经过 Scope Grant；统一的 Identity
不表示所有租户自动看到全部内容。若关系任一端不可读，响应不得泄露该关系或隐藏端点的存在。

`identityId` 由 Runtime descriptor 给出，租户不得自行声明内容归属。真正需要隔离维护与关联的内容应进入
不同 Identity，而不是通过租户名称伪造边界。

### 1.2 Page

Page 是最小的、可独立召回的语义片段，不等于来源系统的一次事件，也不等于一次内容版本：

```json
{
  "pageId": "pg_01...",
  "headRevisionId": "rev_03...",
  "namespace": "project:example",
  "kind": "document",
  "mutability": "sealed|revisioned",
  "lifecycleStatus": "active|superseded|archived|tombstoned",
  "createdAt": "...",
  "updatedAt": "..."
}
```

- `sealed`：原始消息、文件快照、工具结果等事实记录。产生后不得发布第二个 Revision；满足第 2 节
  无损 pack 约束的未引用叶节点可以被一个等价 packed Page 原子替换。
- `revisioned`：Summary、Topic、用户画像、项目状态等系统持续维护的理解。
- `mutability` 是内容不变量，不是历史保留级别。
- `lifecycleStatus` 描述 Page 是否参与默认召回，不表达其内容真假。
- `kind` 是开放字符串，用于路由和 Host 策略；它不产生新的 Core 对象类型。

### 1.3 Revision

Revision 是 Page 的不可变内容快照：

```json
{
  "revisionId": "rev_03...",
  "pageId": "pg_01...",
  "previousRevisionId": "rev_02...",
  "createdAt": "...",
  "observedAt": "...",
  "sourceSpan": { "streamId": "host:chat:main", "start": 41, "end": 41 },
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

`createdAt` 是 Store 提交时间，`observedAt` 是来源事件时间，两者不能合并。可选 `sourceSpan`
标识生产者某条来源流中的闭区间；普通 `ingest_page` 由 Runtime 用认证 Principal 隔离
`streamId`，避免租户误占同一序列。带有 `sourceSpan` 的 Page 即使没有 Relation，仍属于该来源流；
观察工具可以把首尾相接的 span 显示为虚拟邻接，但必须明确区别于持久 Relation 和 provenance。
`sourceSpan` 本身不声称两个事件在语义上相关。

### 1.4 SourceRef 与外部媒体

Page 是 PCP 内稳定的来源身份。原始内容由外部系统保管时，Revision 可以用一个最小 `SourceRef` 指向它：

```json
{
  "providerId": "tenant:photos",
  "locator": "opaque-photo-42",
  "mediaType": "image/jpeg",
  "contentDigest": "sha256:..."
}
```

- `providerId + locator` 只对保管方有解析意义；Runtime 不得把任意路径或 URL 当作受信任抓取指令。
- `mediaType` 和 `contentDigest` 可选。digest 用于核对返回内容，不另造第二套资产身份。
- SourceRef 不承诺原件在线，也不记录可变 availability；解析失败时保留 Page 和已有语义表示。

可检索的 OCR、转写、caption、布局、事件或领域解释应写成普通 Page/Revision，并以 exact provenance
指回承载媒体引用的 Revision。图片不得被强制收缩为唯一 caption；不同任务可以产生多个可追溯表示。
媒体字节托管、Provider 回调与自动提取不属于 v0.8 Core。

### 1.5 Relation

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
`relationType` 是开放字符串，Capability 不枚举词表；以下类型具有 Core 约定语义：

- `summarizes`：来源 Page 是目标 Page 的路由摘要；
- `assesses`：来源 Page 判断目标 Page 当前应如何使用；
- `supersedes`：一个 Page 在语义上替代另一个 Page；
- `aggregates`：来源 Page 聚合多个独立 Page；
- `about`：来源 Page 讨论一个稳定 Topic Page；
- `related_to`：两 Page 存在值得未来联合召回、但没有更准确类型的直接概念关联。

同主题内容优先各自以 `about` 指向同一 Topic Page，不在所有 Page 之间两两建边。`related_to` 是对称兜底
关系：Runtime 按 Page ID 规范化端点，并只保存一个逻辑关系；它不得用于表达顺序、来源、替代或依赖。

`derived_from` 的精确信息应优先写入 Revision provenance；只有确有导航价值时才同时建立
Page Relation。Relation 不能因为时间相邻、同属 Scope、共同命中或向量相似自动产生。
会话顺序属于 Host event stream，不是 Page 图关系。

同一三元组的有效 Relation 必须合并。未经确认的候选只进入维护 ledger，不能直接成为 Relation。错误
Relation 可以撤回；撤回不修改两端 Page。Relation 不会授予另一端 Page 的读取权限。

### 1.6 Provenance

Provenance 属于 Revision，必须引用实际参与生成的精确 `revisionId`。模型或工具生成内容时应
记录操作、Actor、时间、输入 Revision 和必要的工具/模型标识。Page Relation 用于导航，
provenance 用于复现“当时依据了什么”，二者不可互相替代。

仅被检索到、出现在提示上下文或工具结果中、与输出共享 Scope，均不等于实际生成输入，不能写入
provenance。此类可见性若需要保留，应进入访问审计或维护 trace，而不是 Page 图。

## 2. Summary、Validity 与无损 Pack

Summary 是普通的 `revisioned` Page，通过 `summarizes` 指向目标 Page。并非每个 Page 都值得
Summary；只有内容足够长、密集或未来值得路由时才创建。更好的 Summary 更新同一个 Summary
Page，产生新 Revision，而不是制造新 Page。

当一个长期主题跨越多个 Page 时，Runtime maintainer 可以执行
`extract_topic(sourcePages[{pageId, revisionId}], title, content)`。它创建独立的、`kind =
topic_summary` 的 revisioned Topic Page，并为每个输入保留精确 provenance 与从 Topic 到源 Page 的
`summarizes` Relation。输入必须是同一 Scope 内 2–64 个互异、active 的当前 Revision；Topic 不能再作为
Topic 输入。此操作是**逻辑提取**：源 Page 和精确 Revision 不会被删除，仍可通过 ID 读取并在高相关的
Relation 展开中作为证据返回；只是默认 `semantic_search` 和 `match_intent` 的候选面由当前 Topic Page
代表这些源 Page。Topic 更新必须发布新 Revision；只有其当前 Revision 仍列出同一源 Revision 时才继续
压住对应默认候选。

维护 worker 的凝练建议还必须附带简短、来源可核对的理由；它不是写入字段，也不参与 Topic 的恒等性，
仅用于 Console 审阅时解释为什么这一组 Page 值得先被凝练为独立入口。

### 2.1 内容治理：archive 与 restore

`archive_page(page_id, expected_revision_id, reason)` 是受 `manage_lifecycle` 权限保护的人工治理操作。它以
CAS 将 Page 与当前 Revision 从 `active` 同步迁移为 `archived`，并记录操作者、理由、旧/新状态和时间。归档
**不删除** Payload、Revision、Relation、Summary 或 Provenance；精确 `read_pages` 仍可用于审阅，治理接口可以
显式以 `lifecycleStatus=archived` 列表查看。默认 Search、语义/意图召回、图扩展候选和维护库存只消费 active
Page，因此不会通过 archive 再次把内容带入正常上下文。

`restore_archived_page(page_id, expected_revision_id, reason)` 只能将当前 archived Page 恢复为 active，并以相同
的 CAS 和审计约束执行。archive 不是 Topic 提取：Topic 为一个主题建立前置 Page 并保留来源作为高相关证据；
archive 则没有新的检索入口。`purge` 是永久删除，需独立的恢复、保留与确认合同，尚未包含在 v0.8。

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

无损 pack 用于降低大量细粒度 sealed Page 的对象数量，而不让模型重写或删减原始内容。Runtime
可以让语义 worker 从一个有界窗口中选择值得放在一起的有序子序列，但提交由 Store 确定性完成。

一次 `pack_pages` 必须满足：

1. 输入是 2 到 64 个互异的精确当前 Revision，属于同一 Scope、同一 `kind`，并在同一
   `sourceSpan.streamId` 中按请求顺序严格连续；
2. 普通输入必须是 active、sealed、单 Revision Page，且不被 Summary、Validity 或 retention lease
   作为需要保持精确身份的目标；Page Relation、Relation basis 与跨 Revision provenance 不阻止 pack，
   Store 必须在同一事务中把外部连接折叠到 packed Page/Revision，并消解输入之间的内部连接；
3. 输入中可以有至多一个 active、revisioned packed Page 作为稳定锚点；锚点已有的关系、Summary、
   Validity 与历史 Revision 不阻止扩展，因为其 Page 身份不会被销毁；
4. 首次 pack 创建一个新的 revisioned Page；带锚点的 pack 必须以其精确当前 Revision 做 CAS，
   并在同一 Page 上发布一个新 Revision；
5. `application/vnd.pcp.packed-page+json` 的 `entries` 始终是按 sourceSpan 排列的原始叶节点，
   不得包含 packed payload；扩展必须扁平合并，不能形成 pack 嵌套；
6. Store 只删除本次吸收的 sealed Page/Revision，并保留不含正文的 pack ledger。旧精确 ID 必须明确
   报告已被 pack 到哪个 Page，不能静默重定向。旧的锚点 Revision 仍是普通历史 Revision。

pack 销毁被吸收叶节点的 Page 身份，但不丢失其 payload、SourceRef、facets、provenance、Actor 与
时间边界。被吸收节点之间的 Relation/provenance 已由 packed entries 内部表达；连接到输入集合之外的
Relation 端点及 basis、provenance 输入则改指 packed Page/Revision。两个已有 packed Page 不在 v0.8
中直接合并；跨越 sourceSpan 间隙的相关内容应保持独立，并通过 `related_to`、`about`、Topic 或其他
Relation 组织。时间邻近和主题连续性由 Runtime 的语义判断选择，不能削弱 Store 的机械约束。

基于成熟 Summary、Topic 或其他表示**物理删除**原始细节属于有损凝炼。它需要独立的质量、恢复、确认和
审计语义，不属于 v0.8；`extract_topic` 仅改变默认路由，`pack_pages` 也不得被实现成这种操作。

## 3. 接口语义

### 3.1 租户数据面

普通租户的规范接口保持较小：

- `describe() -> identity_id, access, capabilities`
- `list_scopes(query?, limit?, cursor?)`
- `ingest_page(namespace, kind, payload?, source_refs?, observed_at?, source_span?, facets?, external_event_id?)`
- `search_pages(query, scopes, strategy?, limit?, cursor?)`
- `read_pages(page_ids, revision_ids?, view?, max_chars?)`
- `semantic_search(query, scopes?, result_limit?, context_budget_chars?)`
- `match_intent(query, scopes?, result_limit?, context_budget_chars?, intent_effort?)`
- `expand_graph(anchor_page_ids, scopes?, max_depth?, max_nodes?, max_edges?, view?, max_chars?)`
- 可选 `browse_index(scopes?, view?, limit?, cursor?)`

`ingest_page` 是租户唯一的持久写入口。Identity 由所连接的 Runtime 决定，不在 Page、Revision、Scope 或
写请求中重复携带。Runtime 从认证会话填充 Actor、active lifecycle 与 sealed mutability，并隔离可选
`sourceSpan.streamId`；调用方只提供来源事件本身。普通 `read` 会话只能检索和读取；`contribute` 会话
额外获得独立的 `ingest` 权限，但不因此获得高级 Page 写入或维护权限。

Search 返回候选而不是真值，并必须有界、可分页、标明命中投影和当前 Revision。Relation、Summary、
Validity、provenance 与 SourceRef 可以作为被授权的读取投影返回；租户不需要对应的直接写接口。持有精确
Revision ID 的调用方可以按授权读取历史证据，但历史 Revision 不应重新进入默认搜索结果；原始访问审计与
历史枚举仍属于 audit/operator 面。模型决定当前任务需要查询和读取什么，并把结果组装进当前工作上下文。

`search_pages` 是确定性候选和调试接口，不应被作为模型的默认“智能检索”工具。`semantic_search` 是 Runtime
拥有 Provider、预算和组装策略的保守语义入口：它只返回独立相关的 Page，并仅以已断言 Relation 做保守的排序加权。
`match_intent` 可在 `low`、`medium`、`high` 预算内由 Router 扩展意图、审阅候选和关系线索。两者都返回带
`pageId`、`revisionId`、纳入理由、审计与投影内容的结构化条目，**不**返回固定提示词包装；调用方决定如何把这些条目
放入自己的模型上下文。空 `scopes` 表示当前会话的全部授权范围，所有最终读取仍须逐页经过 Store ACL。每个入口未配置
所需 Provider 时必须直接报出对应方法不可用及恢复条件，不得退化为关键词搜索。

`expand_graph` 必须从显式 `anchor_page_ids` 开始，并同时受最大深度（实现上限 3）、节点数和边数约束。它
只返回每一跳均已授权的 Relation/provenance 边与节点；不提供无锚点的全图导出，也不把 Console 的来源流
虚拟边伪装成协议 Relation。`pageId` 是稳定对象身份和图锚点，`revisionId` 是可复核的历史证据定位符。

### 3.2 Runtime 维护面

Runtime maintainer 与本机管理工具可以使用完整 Core 接口：

- `write_page(kind, mutability, content, scope?, based_on_revision_ids?)`
- `revise_page(page_id, expected_revision_id, content, based_on_revision_ids?)`
- `pack_pages(pages[{page_id, revision_id}], idempotency_key?)`
- `extract_topic(source_pages[{page_id, revision_id}], title, content)`
- `archive_page(page_id, expected_revision_id, reason)`
- `restore_archived_page(page_id, expected_revision_id, reason)`
- `write_summary(target_page_id, target_revision_id, content)`
- `assess_validity(target_page_id, target_revision_id, standing, evidence_revision_ids)`
- `relate_pages(from_page_id, relation_type, to_page_id, basis_revision_ids?)`
- `plan_revision_retention(scopes, policy)`
- `collect_revision_retention(scopes, policy, confirmed_revision_ids)`
- `put_revision_retention_lease(revision_id, reason, expires_at, idempotency_key)`
- `list_active_revision_retention_leases(scopes, limit)`

这些操作实现 Identity 范围内的全局维护策略，不属于普通租户合同。实现可以在同一个 RPC transport 上承载
两组操作，但必须以会话权限和操作 allowlist 执行边界；接口分层不要求增加 socket 或部署单元。

### 3.3 控制面与观测面

Runtime Discovery、授权注册、批准与 `open_session` 属于 Runtime 控制协议；Health、Observer snapshot、
原始审计与维护控制属于只读观测或本机 operator 接口。它们可以与 PCP 一同实现，但不是租户 Page 数据面，
也不应通过 Core Page 请求承载。

## 4. 分层责任

协议定义：Identity 边界、Page/Revision 身份、sealed/revisioned 不变量、Page Relation、精确
provenance、SourceRef、Scope 权限、CAS 发布与可追溯回收约束。

Store/Runtime 负责：当前头索引、事务、权限执行、审计、Relation 撤回、历史保留、GC 根、候选发现，
以及 Identity 范围内 Summary、Validity、无损 pack、语义关系和 retention 的全局维护策略。Runtime
可以调用独立模型作为推理 Provider，但必须拥有任务生成、预算、验证、提交与维护 ledger；单个租户不应
成为个人长期上下文的权威维护者。

租户/Host 负责：捕获自己观察到的原始事件、来源内部确定知道的顺序或结构（包括可选 `sourceSpan`）、Page kind、SourceRef 与
外部媒体保管；它可以提交反馈或候选，但不得假定自己拥有其他租户的数据或全局关系图。

消费模型负责：提出查询、读取被授权的精确内容、判断当前任务相关性，并把 PCP 返回的有界结果组装进
当前工作上下文。语义模型 Provider 只向 Runtime 提供判断能力，不直接拥有 Store 写权限或维护 cadence。

PCP 不定义固定 Prompt、向量算法、总结阈值、后台 Agent 拓扑或用户画像格式。

## 5. 保留与回收

实现可以把 Revision 归为当前、受保护、可回收、冷存或缺页占位，但这是 Store 状态，不是
Page/Revision 的协议字段。最小安全规则：

- 当前头永远保留；
- sealed Page 的唯一证据 Revision 默认保留；唯一例外是满足第 2 节全部约束的无损 pack；
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
候选 Revision、只属于候选的投影索引或来源边，以及与这些候选精确关联且已经超过窗口的幂等记录，
并保存不含正文的 collection ledger，使旧 Revision ID 可被识别为“已回收”而不是“从未存在”。
仍可能被 Host 重放的 Page 写入、当前头和其他存活操作，其幂等记录不能仅因时间经过而全局删除。
实现只在 `capabilities.features` 中列出可选能力；保留规划与执行分别使用
`revision_retention_planning` 和 `revision_retention`，支持规划不表示已经支持执行。v0.8 必选能力不再
以恒为 `true` 的布尔字段重复发布。

保留策略按 Identity、Page kind、存储预算和价值配置，不应要求租户或模型为每次写入选择 GC 参数。

## 6. v0.8 版本边界

v0.8 不兼容 v0.7 的 wire 或 Store schema。升级时创建新的 v0.8 Store，并从租户仍持有的原始内容经
`ingest_page` 重新导入；不得直接打开旧数据库，也不得从旧 URI、摘要、向量命中或历史 provenance 猜测
Identity、SourceRef 或新的语义 Relation。高级写接口仍用于 Runtime 维护器与管理工具。
