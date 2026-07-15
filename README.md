# Paged-Context-Protocol (PCP) - v0.4.0-draft

![Paged-Context-Protocol banner](assets/banner.png)

A model-agnostic protocol for a user-owned, cross-session and cross-project
logical context space composed of persistent, addressable Pages.

一个模型无关的上下文协议，以持久、可寻址的 Page 构成由用户拥有的跨会话、跨项目逻辑
上下文空间。

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 简介 (Chinese)

现代模型已经能够自主搜索文件、调用工具并管理当前上下文，但它们通常只能看到当前会话和
当前项目。历史讨论、其他项目、旧任务分支以及用户尚未写入文件的想法，仍然被封锁在模型
可见范围之外。

**Paged-Context-Protocol (PCP)** 为模型提供一个用户拥有的外部逻辑 Page 空间。任何经授权
的模型都可以按照自己的工具习惯搜索、读取、写入、修订和组合 Page，而不需要遵循 PCP
规定的路由、摘要、变焦或推理流程。

### 核心原则

- **模型拥有当前上下文**：窗口内的 working set、搜索计划和压缩策略由模型或 Harness 管理。
- **用户拥有长期上下文**：历史不会绑定在单一模型、服务商、会话或项目目录中。
- **稳定逻辑身份**：Page 与 Revision 拥有稳定 ID，进入或离开模型窗口不改变其身份。
- **原始历史可恢复**：在授权范围内保留可搜索来源；摘要和模型整理是可重建的派生层。
- **显式作用域**：统一地址空间不等于全局注入，跨项目召回必须由 Scope、权限或关系允许。
- **策略中立**：实现可以同时提供 grep、全文、语义、时间和图查询，由模型自行选择。

### PCP 不规定什么

PCP 不再规定四处理器、Intent Focus、固定四级变焦、XML Linear Flow、Chain-of-Thought 或
`Consult/Explore/Shelve/Purge` 状态机。它只定义 Page、Scope、Revision、Provenance、
Relation 和基础上下文 I/O 语义。

### 当前状态

`v0.4.0-draft` 是一次不向后兼容的协议重构。当前重点是确定最小 Core 与接口边界；实现、
跨模型行为和百万级高密度上下文仍需通过评测验证。

旧 `v0.3.0-alpha` 作为一次重要的设计阶段被完整保留，并附有淘汰原因和迁移说明：
[deprecated/v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md)。

---

<a name="english"></a>
## Introduction (English)

Modern models can autonomously search files, call tools, and manage their active
context, but their visibility usually ends at the current session and project.
Past discussions, other projects, old task branches, and ideas that were never
written into files remain outside the model's world.

**Paged-Context-Protocol (PCP)** provides a user-owned external logical Page
space. Any authorized model can search, read, write, revise, and compose Pages
using its own tool habits without following a PCP-defined routing,
summarization, zooming, or reasoning workflow.

### Core Principles

- **The model owns active context**: the model or harness controls its working
  set, search plan, and compaction strategy.
- **The user owns long-term context**: history is not bound to one model,
  provider, session, or project directory.
- **Stable logical identity**: Pages and Revisions keep stable IDs across model
  windows and storage locations.
- **Recoverable raw history**: authorized sources remain searchable; summaries
  and model organization are rebuildable derived layers.
- **Explicit scope**: a unified address space is not global injection;
  cross-project recall requires Scope, permission, or explicit Relations.
- **Strategy neutrality**: Stores may expose grep, full-text, semantic, temporal,
  and graph access surfaces for models to choose among.

### What PCP Does Not Prescribe

PCP no longer mandates four processors, Intent Focus, fixed zoom levels, an XML
Linear Flow, Chain-of-Thought, or a `Consult/Explore/Shelve/Purge` state machine.
It defines only Page, Scope, Revision, Provenance, Relation, and basic context I/O
semantics.

### Current Status

`v0.4.0-draft` is a backward-incompatible redesign focused on the minimal Core
and interface boundary. Implementations, cross-model behavior, and evaluation on
millions of tokens of high-density context remain future work.

The complete `v0.3.0-alpha` generation is preserved as a significant design
stage, together with its deprecation rationale and migration notes:
[deprecated/v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md).

---

## 技术详情 (Technical Specification)

当前协议 / Current specification:
**[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**

历史版本 / Historical generations:
**[deprecated/](deprecated/README.md)**
