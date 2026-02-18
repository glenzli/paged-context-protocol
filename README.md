# Paged-Context-Protocol (PCP) - v0.1.0-alpha

[中文版](#chinese) | [English Version](#english)

---

<a name="chinese"></a>
## 🚀 简介 (Chinese)

**Paged-Context-Protocol (PCP)** 是一种将 LLM 上下文建模为**地址空间（Address Space）**而非单纯“缓存”的管理协议。

与传统的 RAG 或滑动窗口（其本质是将上下文视为不稳定的缓存）不同，PCP 引入了**虚拟内存（Virtual Memory）**的设计哲学。它将碎片化的 Token 流与海量静态数据转化为统一、离散、可寻址的**逻辑页（Logical Pages）**，并允许 Worker 通过动态变焦来控制每一个页面的**展现分辨率**。

### 核心特性
*   **💾 上下文虚拟化与映射 (Mapping)**：将存储索引视为“虚拟磁盘”，实现对话流与大規模静态数据源（Raw Pool）的混合映射。
*   **🔍 需求分页 (Demand Paging)**：由 Worker 发起，实现无限深度的逻辑变焦。
*   **🛡️ 确定性寻址与逻辑主权**：以 XML 标签作为物理地址总线，严禁语义脑补，寻址错误即总线崩溃（Bus Fault）。
*   **🚦 哨兵与逻辑坍缩**：自动监控 Token 压强，通过动态“脱水”维持活跃视界的极高信噪比。

### 为什么选择 PCP？
现有方案本遵循“进场/出场”的**物理缓存逻辑**，而 PCP 遵循“缩放/穿透”的**地址寻址逻辑**。这种视角转变允许模型在有限的窗口内保持对“全域空间”的感知，同时精准定位“局部原子详情”。

---

<a name="english"></a>
## 🚀 Introduction (English)

**Paged-Context-Protocol (PCP)** is a context management protocol that models the LLM context as an **Address Space** rather than a mere "Cache."

While traditional RAG or sliding window approaches treat context as a volatile cache (information is either "in" or "out"), PCP introduces the philosophy of **Virtual Memory**. It transforms fragmented Token streams and massive static data into a unified, discrete, and addressable **Logical Pages** space, allowing the Worker to control the **display resolution** of each page via dynamic zooming.

### Key Features
*   **💾 Context Virtualization & Mapping**: Treats storage as a "Backing Store," enabling hybrid mapping of dialogue flows and massive raw data pools.
*   **🔍 Demand Paging**: Worker-driven recursive mapping, enabling infinite logical zooming depth.
*   **🛡️ Deterministic Addressing & Sovereignty**: XML address bus ensures zero-hallucination; addressing errors are treated as "Bus Faults."
*   **🚦 Sentry & Logic Collapse**: Monitors token pressure and performs dynamic "dehydration" to maintain a high signal-to-noise ratio.

### Why PCP?
Conventional solutions follow a **Physical Cache Logic** (Presence/Absence), whereas PCP follows an **Address Space Logic** (Resolution/Drill-down). This shift enables the model to maintain perception of the "Global Space" within a limited window while precisely locking onto "Local Atomic Details."

---

## 🛠️ 技术详情 (Technical Specification)

详细协议规范请查阅 / Please refer to: **[PROTOCOL.md (CN)](PROTOCOL.md)** | **[PROTOCOL-en.md (EN)](PROTOCOL-en.md)**.
