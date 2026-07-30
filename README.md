# Paged-Context-Protocol (PCP) - v0.4.0-draft

![Paged-Context-Protocol banner](assets/banner.png)

A model-agnostic protocol for paging user-owned information across persistent
Storage, model Memory, and active Context.

一个模型无关的上下文分页协议，让用户拥有的信息在持久 Storage、模型 Memory 与 active
Context 之间保持统一身份、可追溯组织和按需物化。

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 简介 (Chinese)

现代模型能够搜索文件、调用工具并管理当前窗口，但它们通常仍被当前会话和项目边界限制。
完整 Storage 太大，无法直接进入注意力；传统 Memory 往往只保留粗糙压缩，又容易丢失来源、
版本和适用范围。

**Paged-Context-Protocol (PCP)** 定义一个由用户拥有的统一逻辑 Page 空间。原始事件、文件、
历史讨论、模型总结和跨项目关系可以共享稳定身份，并以不同 Projection 按需进入模型注意力。
这里的 Context 是跨时间存在的信息连续体；模型当前窗口只是其中一个临时 working set。

PCP 不是特定的 context manager、Memory 数据库或 Storage 格式。它定义三者之间可互操作、
可追溯的分页边界：

```text
Source / Event
  -> Page / Revision             (Storage)
  -> Summary / Derived DAG       (Memory)
  -> Search / Read Projection    (Attention boundary)
  -> Active working context
```

其中 Memory 表示 PCP 内部的派生组织层，不要求部署独立的 Memory 服务或 Profile。

### 核心原则

- **用户拥有信息连续性**：历史不会绑定在单一模型、服务商、会话或项目目录中。
- **稳定逻辑身份**：Page 与 Revision 拥有稳定 ID，进入或离开模型窗口不改变其身份。
- **原始历史可恢复**：在授权范围内保留可搜索来源；摘要和模型整理是可重建的派生层。
- **稀疏模型记忆**：值得索引的 Page 可以拥有独立、可版本化的 Summary Projection，模型再
  按需读取 Detail。
- **可回溯整理图**：总结与聚合形成无环派生子图，并能逐层返回精确来源 Revision。
- **显式注意力物化**：已返回候选、Summary 与 Detail 必须以明确的 Page、Revision 和
  Projection 身份进入模型可见上下文。
- **显式作用域**：统一地址空间不等于全局注入，跨项目召回必须由 Scope、权限或关系允许。
- **模型拥有策略**：搜索计划、排序、压缩和具体准入时机仍由模型或 Host 决定。

### PCP 不规定什么

PCP 不再规定四处理器、Intent Focus、固定四级变焦、XML Linear Flow、Chain-of-Thought 或
`Consult/Explore/Shelve/Purge` 状态机。它定义 Page、Scope、Revision、Provenance、
Relation、Summary Projection、Detail 读取、attention materialization 和基础 I/O 语义，
但不规定模型必须采用哪条召回路径，也不替模型决定某项内容何时最值得进入注意力。

### 当前状态

`v0.4.0-draft` 是一次不向后兼容的协议重构，目前正由实际工程中的原型持续反哺。Core 与
接口边界仍可能随实践调整；跨模型行为和百万级高密度上下文仍需系统评测。

旧 `v0.3.0-alpha` 作为一次重要的设计阶段被完整保留，并附有淘汰原因和迁移说明：
[deprecated/v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md)。

---

<a name="english"></a>
## Introduction (English)

Modern models can search files, call tools, and manage an active window, but
their visibility usually still ends at the current session and project. Complete
Storage is too large to place directly into attention, while conventional Memory
often preserves only coarse compression and loses source, revision, or scope.

**Paged-Context-Protocol (PCP)** defines a user-owned unified logical Page space.
Raw events, files, past discussions, model summaries, and cross-project
Relations can share stable identities and enter model attention through
different Projections on demand. Context here is an information continuum across
time; the model's current window is only one temporary working set.

PCP is not a specific context manager, Memory database, or Storage format. It
defines an interoperable and traceable paging boundary among them:

```text
Source / Event
  -> Page / Revision             (Storage)
  -> Summary / Derived DAG       (Memory)
  -> Search / Read Projection    (Attention boundary)
  -> Active working context
```

Here, Memory names PCP's derived organization layer; it does not require a
separate Memory service or profile.

### Core Principles

- **The user owns information continuity**: history is not bound to one model,
  provider, session, or project directory.
- **Stable logical identity**: Pages and Revisions keep stable IDs across model
  windows and storage locations.
- **Recoverable raw history**: authorized sources remain searchable; summaries
  and model organization are rebuildable derived layers.
- **Sparse model memory**: Pages worth indexing may have independently
  versioned Summary Projections from which a model can read Detail on demand.
- **Traceable organization graph**: summarization and aggregation form an
  acyclic derivation subgraph that leads back to exact source Revisions.
- **Explicit attention materialization**: returned candidates, Summaries, and
  Detail enter model-visible context with explicit Page, Revision, and
  Projection identities.
- **Explicit scope**: a unified address space is not global injection;
  cross-project recall requires Scope, permission, or explicit Relations.
- **The model owns strategy**: search planning, ranking, compaction, and exact
  admission timing remain Model Client or Host policy.

### What PCP Does Not Prescribe

PCP no longer mandates four processors, Intent Focus, fixed zoom levels, an XML
Linear Flow, Chain-of-Thought, or a `Consult/Explore/Shelve/Purge` state machine.
It defines Page, Scope, Revision, Provenance, Relation, Summary Projections,
Detail reads, attention materialization, and basic I/O semantics without
prescribing a recall path or deciding when a specific item is most worthy of
model attention.

### Current Status

`v0.4.0-draft` is a backward-incompatible redesign continuously informed by an
implementation prototype in a real project. Core and interface boundaries may
still change with practice; cross-model behavior and millions-of-token,
high-density context require systematic evaluation.

The complete `v0.3.0-alpha` generation is preserved as a significant design
stage, together with its deprecation rationale and migration notes:
[deprecated/v0.3.0-alpha](deprecated/v0.3.0-alpha/README.md).

---

## 技术详情 (Technical Specification)

当前协议 / Current specification:
**[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**

历史版本 / Historical generations:
**[deprecated/](deprecated/README.md)**
