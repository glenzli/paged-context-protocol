# Paged-Context-Protocol (PCP) - v0.1.0-alpha

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 🚀 简介 (Chinese)

**Paged-Context-Protocol (PCP)** 是一种将 LLM 上下文建模为**地址空间（Address Space）**而非单纯“线性缓存”的指令集架构协议。

在本协议中，LLM 不再是简单的文本生成器，而是被解构为具备**逻辑虚拟内存（LVM）**管理能力的**逻辑处理器（Logic Processor）**。PCP 将碎片化的 Token 流与海量原始数据转化为统一、离散、可寻址的**逻辑页（Logical Pages）**，并通过**递归逻辑树（Logic Trees）**拓扑实现对海量语境的精准变焦与探测。

### 核心特性
*   **💾 逻辑虚拟内存 (LVM)**：将物理存储（PBlock）与逻辑地址空间（LAS）解耦，支持超长历史与海量文件的混合变焦映射。
*   **🔍 指令驱动寻址 (ISA)**：定义 `Consult`、`Explore`、`Shelve` 指令。由 Processor 自主驱动物理探测与逻辑解析，实现“按需分页”。
*   **🌲 递归逻辑树 (Logic Trees)**：地址空间从“扁平流”进化为“树形分支”，支持通过 `Unpacked` 状态进行无限深度的纵向逻辑下钻。
*   **⚡ 投机物化与草稿页**：引入 **Draft Pages** 机制，支持在不物化全文的情况下感知识识读物理块的逻辑分布。
*   **🛡️ 总线主权与低熵保护**：以 XML 作为物理寻址总线，严禁语义幻觉。内置**低熵保护策略**，自动重构模糊意图，确保寻址的确定性。

### 为什么选择 PCP？
现有方案遵循“缓存置换”的**物理逻辑**，而 PCP 遵循“缩放/穿透”的**处理器逻辑**。这种视角转变允许模型在有限的窗口内保持对“全域空间”的逻辑连续性感知，将 LLM 转化为真正的长文本执行引擎。

### 理论基础

PCP 对 LLM 幻觉问题（特别是 **Type IV 注意力预算约束**）的缓解已有形式化数学分析，详见 [llm-logic-fragments / Type IV — 注意力预算的结构性约束](https://github.com/glenzli/llm-logic-fragments/blob/main/hallucination/type-iv-attention-dilution.md)，核心结论如下：

| 命题 | 内容 | 证明状态 |
|---|---|---|
| A — 有效竞争者数量减少 | PCP 将 Worker 所见的有效 token 数压缩为 $N_\text{eff} \leq N_\text{hot} + r \cdot N_\text{raw}$，$r < 1$ | ✅ 严格 |
| B — 循环依赖被架构打破 | Router 独立承担路由，Worker 不依赖自身注意力决定上下文组成 | ✅ 严格 |
| C — Router 自身避免严重 IV-a | Router 处理页面索引规模 $P \ll N_\text{raw}$，稀释程度远低于无 PCP 时 | ✅ 严格 |
| F/G — 时间维度不变性 | PCP Memory 使历史信息可达性与时间距离 $\Delta$ 无关 | ✅ 严格 |

> [!NOTE]
> 分析同时涵盖 **IV-a（位置注意力稀释）** 与 **IV-b（特征注意力误路由）**。`Shelve`/`Purge` 指令在 IV-b 的 SNR 框架下起信号增强 + 噪声抑制的双重作用。

---

<a name="english"></a>
## 🚀 Introduction (English)

**Paged-Context-Protocol (PCP)** is an Instruction Set Architecture (ISA) protocol that models LLM context as an **Address Space** rather than a mere "linear cache."

Under PCP, the LLM is no longer just a text generator but is deconstructed into a **Logic Processor** with **Logic Virtual Memory (LVM)** management capabilities. PCP transforms fragmented Token streams and massive raw data into a unified, discrete, and addressable **Logical Pages** space, utilizing a **Hierarchical Logic Tree** topology for precise context zooming and probing.

### Key Features
*   **💾 Logic Virtual Memory (LVM)**: Decouples physical storage (PBlocks) from the Logical Address Space (LAS), enabling hybrid mapping of massive data sources and long-term history.
*   **🔍 Instruction-Driven Addressing (ISA)**: Defines `Consult`, `Explore`, and `Shelve` instructions. The Processor autonomously drives physical probing and logical parsing, achieving "Demand Paging."
*   **🌲 Recursive Logic Trees**: Evolves context from a "flat stream" into "hierarchical branches," supporting infinite vertical drill-down via the `Unpacked` state.
*   **⚡ Speculative Materialization & Draft Pages**: Introduces **Draft Pages** for perceiving the logical distribution of physical blocks without full materialization.
*   **🛡️ Bus Sovereignty & Low-Entropy Protection**: Uses XML as the physical addressing bus to eliminate hallucinations. Includes a **Low-Entropy Protection** strategy to automatically reconstruct ambiguous intents.

### Why PCP?
Conventional solutions follow the **Physical Logic** of cache replacement, whereas PCP follows the **Processor Logic** of resolution and penetration. This shift enables the model to maintain logical continuity across the "Global Space" within a limited window, transforming the LLM into a true long-context execution engine.

### Theoretical Grounding

The formal mathematical analysis of PCP's mitigation of LLM hallucinations (specifically **Type IV: Attention Budget Structural Constraints**) is documented in [llm-logic-fragments / Type IV — Attention Budget Structural Constraints](https://github.com/glenzli/llm-logic-fragments/blob/main/hallucination/type-iv-attention-dilution.md). Key conclusions:

| Proposition | Content | Proof Status |
|---|---|---|
| A — Effective competitor reduction | PCP compresses Worker's effective token count to $N_\text{eff} \leq N_\text{hot} + r \cdot N_\text{raw}$, $r < 1$ | ✅ Strict |
| B — Cyclic dependency broken by architecture | Router handles routing independently; Worker never depends on its own attention to determine context composition | ✅ Strict |
| C — Router avoids severe IV-a | Router operates at page-index scale $P \ll N_\text{raw}$; attention dilution far lower | ✅ Strict |
| F/G — Temporal invariance | PCP Memory makes historical info recall independent of time distance $\Delta$ | ✅ Strict |

> [!NOTE]
> The analysis covers both **IV-a (Positional Attention Dilution)** and **IV-b (Feature Attention Misrouting)**. `Shelve`/`Purge` instructions act as dual-mode signal amplifiers + noise suppressors in the IV-b SNR framework.

---

## 🛠️ 技术详情 (Technical Specification)

详细协议规范请查阅 / Please refer to: **[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**.
