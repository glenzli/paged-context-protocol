# Paged-Context-Protocol (PCP) v1.0

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 🚀 简介 (Chinese)

**Paged-Context-Protocol (PCP)** 是一种将 LLM 上下文建模为**地址空间（Address Space）**而非单纯“缓存”的管理协议。

与传统的 RAG 或滑动窗口（其本质是将上下文视为不稳定的缓存）不同，PCP 引入了**虚拟内存（Virtual Memory）**的设计哲学。它将碎片化的 Token 流转化为离散、可寻址的**逻辑页（Logical Pages）**，并允许 Worker 通过动态变焦来控制每一个页面的**展现分辨率**。

### 核心特性
*   **💾 上下文虚拟化**：将存储索引视为“虚拟磁盘”，将物理上下文窗口视为“一级缓存 (L1 Cache)”。
*   **🔍 需求分页 (Demand Paging)**：通过 `Consult` 算子实现类似 Page Fault Handler 的按需加载，动态调取细节。
*   **🛡️ 确定性寻址**：使用 XML 支架作为逻辑寻址的总线结构，防止模型在长程推理中产生地址偏移（幻觉）。
*   **⚖️ 三位一体内核**：Router（MMU/寻址）、Worker（CPU/执行）、Consolidator（GC/后台整理）。

### 为什么选择 PCP？
现有方案本遵循“进场/出场”的**物理缓存逻辑**，而 PCP 遵循“缩放/穿透”的**地址寻址逻辑**。这种视角转变允许模型在有限的窗口内保持对“全域空间”的感知，同时精准定位“局部原子详情”。

---

<a name="english"></a>
## 🚀 Introduction (English)

**Paged-Context-Protocol (PCP)** is a context management protocol that models the LLM context as an **Address Space** rather than a mere "Cache."

While traditional RAG or sliding window approaches treat context as a volatile cache (information is either "in" or "out"), PCP introduces the philosophy of **Virtual Memory**. It transforms fragmented Token streams into discrete, addressable **Logical Pages**, allowing the Worker to control the **display resolution** of each page via dynamic zooming.

### Key Features
*   **💾 Context Virtualization**: Treats long-term storage as "Disk" and the physical context window as an "L1 Cache."
*   **🔍 Demand Paging**: Implements `Consult` as a Page Fault Handler to load details on-demand without losing global awareness.
*   **🛡️ Deterministic Addressing**: Uses XML scaffolding as a logical address bus to prevent "address drift" (hallucination) during long-range reasoning.
*   **⚖️ Trio Kernel Model**: Separation of duties between the Router (MMU/Adressing), Worker (Execution), and Consolidator (Background GC/Refinement).

### Why PCP?
Conventional solutions follow a **Physical Cache Logic** (Presence/Absence), whereas PCP follows an **Address Space Logic** (Resolution/Drill-down). This shift enables the model to maintain perception of the "Global Space" within a limited window while precisely locking onto "Local Atomic Details."

---

## 🛠️ 技术详情 (Technical Specification)

详细协议规范请查阅 / Please refer to: **[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**.
