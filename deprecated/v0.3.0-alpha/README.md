# PCP v0.3.0-alpha - Deprecated Design Record

> **Status: Deprecated.** This directory is a historical snapshot and must not
> be treated as the current PCP specification.

> **状态：已淘汰。** 本目录是历史设计快照，不再作为当前 PCP 实现规范。

## 保存内容

- [PROTOCOL.md](PROTOCOL.md)：v0.3.0-alpha 中文协议。
- [PROTOCOL-en.md](PROTOCOL-en.md)：v0.3.0-alpha English specification.
- [memory/SPEC.md](memory/SPEC.md)：与该协议配套的旧 P-Mem Profile。

这些文件被整体迁移到同一目录，以保留当时的相对链接、术语和设计边界。除增加弃用标记
或修复严重的历史文档断链外，本快照不再继续修改。

## 为什么被淘汰

v0.3.0-alpha 形成于模型需要大量显式脚手架才能稳定管理长上下文的阶段。它不仅定义
Page 的身份和持久化语义，还规定了模型应当如何执行上下文管理，包括：

- Router、Worker、Consolidator、Auditor 四处理器；
- Intent Focus、两阶段 Schema CoT 与固定路由流程；
- `Summary -> Detail -> Unpacked` 的递归变焦状态机；
- `Consult`、`Explore`、`Shelve`、`Purge` 指令；
- `<Linear_Flow>`、`<Reasoning_Trace>` 与 XML 合成；
- 由协议主动控制窗口压缩、内容驻留和上下文折叠。

这套设计帮助澄清了上下文虚拟化、逻辑页、按需召回和同源记忆等概念，但后来暴露出
以下根本问题。

### 1. 将持久化协议与模型运行策略混为一体

不同模型具有不同的工具使用习惯。模型能力提升后，它们已经能够自行决定何时搜索、
使用 grep 还是语义检索、读取哪些来源以及如何维护当前 working set。协议继续规定
统一的路由和变焦流程，会增加 Token、延迟和实现耦合，却不能保证更好的结果。

### 2. 固定变焦层级成为仪式，而不是必要语义

Summary 可以只是 Page 的可选 facet，Detail 可以只是读取 payload，Unpacked 可以只是
遍历关系。将它们建模为强制运行时状态，给模型和 Host 引入了不必要的状态组合。

### 3. Summary 中心的持久化仍会重演压缩损失

跨项目、跨多年和高密度知识无法可靠地压缩为少量摘要。写入时被判断为次要的条件，
可能在很久以后成为关键依赖。摘要应当是可重建的索引或派生视图，不能代替原始历史、
来源和稳定逻辑身份。

### 4. 模型真正缺少的是窗口之外的可见性

现代模型能够搜索当前项目文件，却通常无法访问历史讨论、其他项目、旧任务分支以及
用户曾经提出但尚未写入文件的想法。问题已经从“如何教模型压缩当前窗口”转变为
“如何让任何模型访问用户拥有的跨会话、跨项目逻辑地址空间”。

### 5. 部分核心字段承担了互相冲突的职责

例如 `content_mode` 同时被当作 Page 属性与单次 Fetch 的返回状态；`residency` 被建模为
单值，但同一 Page 可以同时存在于持久存储和多个运行窗口；旧 `trust` 枚举也混合了
来源、指令权限、完整性和事实可靠性。这些问题需要从数据模型层重构，而非继续增加
运行时规则。

## 仍然有效的贡献

本版本没有因为模型增强而失去全部价值。以下原则被保留并成为后续协议的基础：

- 上下文应当具有稳定、可寻址的逻辑身份；
- 当前窗口只是 Page 的临时投影，不是 Page 的所有权边界；
- 当前上下文与历史上下文应共享同一套地址语义；
- 摘要不是最终证据，内容必须能够回溯到来源；
- 外部存储应支持按需读取，而不是把全部历史重新注入窗口；
- 用户的长期上下文不应绑定于单一模型或单一厂商。

## 后继方向

当前协议将 PCP 重新定位为：

> **供模型自主操作的、用户拥有的跨会话与跨项目逻辑 Page 空间。**

新协议只规定 Page、Scope、Revision、Provenance、Relation 和基础 I/O 语义，不再规定
模型的搜索计划、摘要层级、上下文压缩方法或推理流程。参见：

- [当前中文草案](../../PROTOCOL.md)
- [Current English Draft](../../PROTOCOL-en.md)

## 概念迁移映射

| v0.3.0-alpha | v0.4.0-draft |
| :--- | :--- |
| Quad Processor Model | Model Client、Host、Store、Adapter 的逻辑边界；不规定部署拓扑 |
| Original / Consolidated Page | Source-backed / Derived Page；类型词汇保持开放 |
| `Summary / Detail / Unpacked` | 可选 Facets、Read Projection 与 Relation traversal |
| Intent Focus | 普通 `SearchPages` query；可由模型自由分解 |
| `Consult` / `Explore` | `ReadPages` / `SearchPages` |
| `Shelve` | 模型或 Harness 的 active-context 管理，不属于 PCP Core |
| `Purge` | task-scoped `SuppressPages`、durable Tombstone 或受控 Delete |
| `<Linear_Flow>` / XML | Host 生成的可选 Context Bundle 或任意传输表示 |
| 独立 P-Mem Profile | 持久 Page Store 成为 PCP Core 本身 |
| 固定 Router 与检索阶段 | Store 暴露多种访问表面，模型自行选择和组合 |

## English Summary

PCP v0.3.0-alpha coupled a durable logical page store with a prescriptive model
runtime: four processors, Intent Focus routing, fixed zoom states, protocol
instructions, and XML context synthesis. This coupling became unnecessary as
models learned to search, inspect, and manage their active context through tools
without a protocol-defined workflow.

The generation remains historically important because it established logical
pages, demand retrieval, evidence traceability, and same-origin memory. Its
successor keeps those invariants while moving routing, summarization, compaction,
and context assembly out of the normative core. PCP now defines a model-agnostic,
user-owned backing store that any capable model may navigate through ordinary
interfaces.
