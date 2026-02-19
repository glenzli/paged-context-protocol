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

---

## 🛠️ 技术详情 (Technical Specification)

详细协议规范请查阅 / Please refer to: **[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**.
