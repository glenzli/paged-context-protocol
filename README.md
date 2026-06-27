# Paged-Context-Protocol (PCP) - v0.2.0-alpha

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 简介 (Chinese)

**Paged-Context-Protocol (PCP)** 是一种将 LLM 上下文建模为**地址空间（Address Space）**而非单纯线性缓存的上下文治理协议。

PCP 将碎片化的 Token 流与外部数据源转化为离散、可寻址的**逻辑页（Logical Pages）**，并通过**递归逻辑树（Logic Trees）**、按需下钻和后台整理，在有限上下文窗口内提高召回、溯源和噪声控制能力。

### 核心特性
*   **逻辑地址空间 (LAS)**：将物理存储（PBlock）与逻辑页面（Logical Pages）解耦，支持长历史、文件和仓库的统一寻址。
*   **指令驱动寻址**：定义 `Consult`、`Explore`、`Shelve`、`Purge` 等指令，用于按需下钻、物理探测、上下文折叠和错误召回隔离。
*   **递归逻辑树 (Logic Trees)**：通过 `Summary`、`Detail`、`Unpacked` 视图在章节、主题或页面层级之间切换。
*   **草稿页 (Draft Pages)**：在不直接注入全文的情况下，让 Worker 感知候选物理块的逻辑分布。
*   **结构化上下文合成**：以 XML 或等价结构化格式提供页面边界、ID、时间戳和信任标注，降低上下文混淆风险。

### 为什么选择 PCP？
普通长上下文、滑动窗口、一次性 RAG 和滚动摘要都容易在长程任务中丢失低频但关键的逻辑锚点。PCP 的目标是让上下文成为可寻址、可下钻、可折叠、可追溯的系统状态，而不是每轮临时拼接的一段文本。

### 验证状态

PCP 目前是协议与工程设计草案。它提出的效果应通过实现和评测验证，例如证据召回率、上下文污染率、摘要损失率、`Consult` 成功率、成本和延迟。

### 同源记忆层

PCP-native Memory profile 已并入本仓库，见 [memory/SPEC.md](memory/SPEC.md)。它定义了同源持久化 Page Store 的基本契约，包括 `QueryMemory`、`FetchMemory`、`content_mode`、`available_modes`、版本与溯源。

---

<a name="english"></a>
## Introduction (English)

**Paged-Context-Protocol (PCP)** is a context governance protocol that models LLM context as an **Address Space** rather than a linear cache.

PCP transforms fragmented token streams and external data sources into discrete, addressable **Logical Pages**. It uses recursive logic trees, demand-driven drill-down, and background consolidation to improve recall, traceability, and noise control within a finite context window.

### Key Features
*   **Logical Address Space (LAS)**: Decouples physical storage (PBlocks) from addressable logical pages.
*   **Instruction-Driven Addressing**: Defines `Consult`, `Explore`, `Shelve`, and `Purge` for drill-down, physical probing, context folding, and negative feedback.
*   **Recursive Logic Trees**: Switches between `Summary`, `Detail`, and `Unpacked` views across topic and page levels.
*   **Draft Pages**: Exposes the logical distribution of candidate physical blocks without immediately injecting full source content.
*   **Structured Context Synthesis**: Uses XML or an equivalent structured format to preserve page boundaries, IDs, timestamps, and trust labels.

### Why PCP?
Long-context models, sliding windows, one-shot RAG, and rolling summaries can still lose low-frequency but critical logical anchors in long-horizon tasks. PCP aims to make context addressable, drillable, foldable, and auditable instead of rebuilding it as ad hoc text each turn.

### Validation Status

PCP is currently a protocol and engineering design draft. Its claims should be validated empirically with implementation benchmarks such as evidence recall, context pollution, summary loss, `Consult` success rate, cost, and latency.

### Same-Origin Memory Layer

The PCP-native Memory profile now lives in this repository. See [memory/SPEC.md](memory/SPEC.md) for the same-origin persistent Page Store contract, including `QueryMemory`, `FetchMemory`, `content_mode`, `available_modes`, versioning, and provenance.

---

## 技术详情 (Technical Specification)

详细协议规范请查阅 / Please refer to: **[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)** | **[memory/SPEC.md](memory/SPEC.md)**.
