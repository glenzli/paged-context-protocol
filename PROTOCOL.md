# Paged-Context-Protocol (PCP) - v0.1.0-alpha

Paged-Context-Protocol (PCP) 是一种专注于 LLM **统一逻辑寻址**与**分布式上下文治理**的底层协议。它将 LLM 视为一种**柔性执行的逻辑 CPU (Flexible Logic CPU)**，通过将碎片化的对话流与海量异构数据源（文件、流、仓库）统一映射为离散、可寻址的**逻辑页（Logical Pages）**，从物理上解决了长程交互中的上下文污染与信息过载，赋予模型在无限地址空间内行使**逻辑主权**的能力。

## I. 核心哲学：柔性算力与逻辑虚拟内存 (Core Philosophy)

PCP 协议将 LLM 视为一种具备**柔性执行能力的逻辑 CPU (Flexible Logic CPU)**，而将本规格定义为该处理器的**逻辑虚拟内存 (Logical Virtual Memory, LVM)** 协议。其核心目标是实现智能逻辑与物理载体的深度解耦。

### 1.1 线性逻辑流 vs. 外部系统 (Linear Logic vs. External)
PCP 定义了两类外部系统，并对其有不同的耦合要求：
*   **RAG 系统 (Generic Cold Storage)**：泛化的、非线性的外部数据库。PCP **完全不关心**其内部设计，仅将其视为 PBlock 的物理提供方。
*   **记忆系统 (Memory - Logic Extended Cache)**：PCP 的**逻辑扩展缓存**。Memory 是 PCP 的关键“外挂”，它存储经由 PCP 处理后的结构化逻辑。PCP 对其有**强耦合要求**：Memory 必须能基于语义关键词给出符合 PCP 页面定义的高相关性内容，作为 Router 意图匹配的恒定输入。
*   **PCP 协议 (Linear Logic Stream)**：则是**意图驱动的“运行热流”**。它在任务 Timeline 上处理逻辑闭合，利用 Memory 提供的历史逻辑资产来辅助当前的推演。

### 1.2 核心支柱
*   **角色重构 (LLM as CPU)**：不再将 LLM 视为存储事实的“百科全书”（Memory-heavy），而是将其定义为专注于指令执行与逻辑流管理的**逻辑处理器 (Logic Processor)**。其核心任务是处理 Page 间的拓扑关系，而非单纯的文本生成。
*   **上下文虚拟化 (Context Virtualization)**：将所有逻辑资产（历史、文档、代码）视为“后台存储”。物理窗口（Context Window）仅作为展示高分辨率细节的“热缓存 (L1/L2 Cache)”，实现“无限长”的逻辑感知。
*   **统一寻址逻辑 (Unified Addressing)**：通过 Pages 实现“内存与存储”的逻辑合一。无论是即时对话还是海量归档，均在同一套逻辑地址空间（LAS）内进行统一坐标定标与调度。
*   **按需分页 (Demand Paging)**：Worker 不应被动承载所有信息，而应动态行使**映射权 (Mapping)**，通过 **Consult** 指令在地址空间中自主按需调取深层细节。
*   **逻辑主权 (Logical Sovereignty)**：PCP 赋予执行体绝对的判定权力。处理器作为逻辑单元，严禁对地址空间外的未知信息进行“语义脑补”，所有逻辑缺口必须通过协议指令触发物理穿透（Zooming）解决。


## II. 逻辑处理器模型 (Trio Processor Model)

系统基于三个核心角色的解耦协作运行，确保“治理”与“执行”的分离：

1.  **寻址处理器 (Router-MMU)**: 
    *   **职责**: 逻辑坐标映射（Logical Mapping）。负责意图识别、逻辑页面索引、两阶段相关性匹配。
    *   **意图锚定**: 通过分析活跃上下文的“Head”（系统指令/全局上下文）与“Tail”（当前用户查询），提取当前**意图重心 (Intent Focus)**。
    *   **核心特征**: **神经寻址优于数值检索**。Router 利用高维语义空间评估逻辑相关性，并生成**寻址指令**驱动 Mapping。

2.  **执行处理器 (Worker-CPU)**: 
    *   **职责**: 任务执行与**即时映射 (JIT Mapping)**。Worker 不再是简单的内容生成器，而是被解构为具备高维逻辑感知能力的执行单元，执掌逻辑推演并行使映射主权。
    *   **映射主权**: 作为逻辑主权的行使者，Worker 负责解构和映射 Host 提供的**原始物理块 (Raw PBlocks)** 为逻辑页面。
    *   **核心动作**: 动态确定物理数据的逻辑边界；触发“按需分页”指令；执行变焦与扩散算法。

3.  **整理处理器 (Consolidator-Background GC)**: 
    *   **职责**: 系统存储空间的后台维护（Memory Manager）。
    *   **核心动作**: 
        1.  **初始固化 (Freezing)**：监听话题状态与长度阈值。其“话题转折（Topic Pivot）”的判定是基于推理逻辑的突变，而非简单的语义距离。
        2.  **长效整合 (Merging)**：基于逻辑陈旧度执行“代谢合并”。

### 2.4 PCP 处理器效能基准 (Processor Proficiency Baseline)

PCP 协议将逻辑寻址与状态控制高度解耦并下放至执行器。作为处理器的 LLM 必须满足以下“逻辑物理”底线，以确保在复杂环境下进行有效的逻辑推演：
*   **指令一致性 (Instruction Consistency)**：模型必须具备极高的**结构化输出鲁棒性**。协议定义的标记（如当前采用的 XML 标签）必须被严格遵守，视为“物理边界”。任何失控的语法截断、格式错位或结构性破坏，都将被视为**总线故障 (Bus Fault)**，强制中断协议任务。
*   **语义熵压缩 (Semantic Entropy Compression)**：执行整理 (Consolidation) 时，模型必须保持“**逻辑锚点无畸变**”。在压缩文本的同时，必须保留核心推演链条和物理标识符（如 ID、变量、参数）。“文学化摘要”将被视为 PCP 中的**载荷错误 (Payload Error)**。
*   **主动压力感知 (Proactive Pressure Awareness)**：Worker 必须具备**“逻辑真空感知”**能力。当当前视界无法闭合逻辑链条时，必须精确触发 `Consult` 指令。在分辨率不足的情况下执行“语义填充”（幻觉）是被严格禁止的。主动**变焦 (Zooming)** 优先于盲目推演。

### 2.5 异构与并行部署策略

PCP 的三处理器架构支持 **异构双模型 + 并行部署 (Dual-LLM Parallel Strategy)**：

1.  **寻址层 (Router)**：
    *   **建议**：采用轻量级、高吞吐的高效模型。由于寻址属于“相对语义任务”，对吞吐量的要求远高于推理精度。
    *   **未来提案 (Proposal)**：随着 **1.58-bit 极端量化模型**（如 BitNet b1.58）的成熟，Router 是本地化部署该类模型的理想场景，可实现近乎零成本、极高频次的逻辑寻址协同。
2.  **逻辑层 (Worker + Consolidator)**：
    *   **特性**：二者**必须共享同一款高性能主模型**（Main LLM）。由于 JIT Mapping 与 Consolidation 均属于对数据进行“绝对逻辑提取”的任务，必须使用同一套逻辑度量衡以防止语义漂移。
    *   **并行部署 (Parallel Instances)**：虽然模型一致，但物理上应通过并行实例部署。
        - **Worker (同步)**：响应用户主交互链路，确保即时性。
        - **Consolidator (异步)**：作为“后台 GC 进程”运行在独立并行实例中。它由 Host 监听 Token 压强或话题转折（Topic Pivot）后反应式触发，在后台完成索引合并与固化，从而避免后台维护任务阻塞用户主循环。

这种“高效寻址 + 逻辑一致主模型 + 异步维护并行化”的部署方案，在确保系统逻辑闭合的同时，实现了极佳的响应性能。


## III. 宿主系统模型 (The Host System Model)

宿主系统（Host）提供协议运行的“刚性”物理环境。它作为 PCP 的基础设施，负责资源管理与报文合成，确保协议的物理安全性与逻辑一致性：

1.  **索引引擎 (Index Manager)**:
    *   **Page 注册与 ID 分配**：负责 Logical Address Space (LAS) 内所有页面的全球唯一 `Short Hash` 分配。
    *   **热度与状态追踪**：维护页面在索引中的激活态（Hot/Indexed/Shelved）及逻辑权重。
    *   **PBlock 全权管控**：负责物理分块的寻址句柄分配及物理资源的冷热交换。

2.  **合成控制器 (Synthesis Controller / XML Engine)**:
    *   **视界动态流水线**：根据寻址结果，实时合成用于注入 Worker 处理器的 XML 报文。
    *   **物理溢出治理 (Overflow Governance)**：此为核心守护逻辑。当 Worker 触发 `Consult` 或 `Mapping Pulse` 导致新 Page 注入时，控制器必须动态评估窗口压强。若越限风险增加，控制器强制对非意图重心的 Page 执行“动态脱水”——即在注入前强制执行 `view="Detail" -> "Summary"` 的降阶处理。

3.  **总线中继器 (Bus Mediator)**:
    *   **指令翻译层**：接收 Worker 输出的 `Consult(id)` 或 `Shelve(id)` 等指令，并将其翻译为具体的物理存储读取（I/O）或索引状态修改动作。
    *   **原子性保障**：确保每一轮指令执行的完整性，防止因局部读取失败导致逻辑状态机挂死。

4.  **意图脚手架 (Intent Scaffolding)**:
    *   **逻辑自愈**：在用户输入熵极低的情况下，自动提取前序 Page Summary 辅助意图重构，确保 Worker 始终在闭合的逻辑平面内工作。

5.  **统一导出管理器 (Unified Export Manager)**:
    *   **逻辑净值导出**：监听 Consolidator 的提取动作或任务结束信号。
    *   **职责边界**：PCP 不负责决策知识的最终去向。导出管理器仅通过 `Export_Logic_NetValue` 接口提供高质量的逻辑资产（如 Consolidated Page Summary、Keywords）。
    *   **无状态处理**：导出过程是异步且单向的，PCP 将逻辑资产交付给外部系统后即完成职责，由外部系统自主决定是存入 Memory 还是沉淀至 RAG。


## IV. 时间定标系统 (The Temporal Coordination System)

PCP 采用**时间轴 (Timeline)** 作为逻辑排序与关注度引导的核心规范：

*   **时间戳锚定**: 每一个逻辑页（Original/Consolidated）都必须携带绝对时间戳锚点。对于静态存储块，采用其**物理修改时间**或**逻辑加载序号**作为定标。
*   **当前时间注入**: 在每次交互的上下文顶部，显式注入 `Current_Time`。
*   **时序感知**: Worker 通过对比 Page 时间戳与 `Current_Time` 判断逻辑的先后顺序或数据的陈旧度。

## V. 逻辑实体与地址空间 (Logical Entities & Address Space)

PCP 维护两个层级的地址空间，通过 Worker 的映射行为实现交互：

### 5.0 物理存储块 (Physical Block, PBlock) — PAS 层
*   **职责**: 代表物理地址空间 (Physical Address Space, PAS)。由 **Host (系统)** 负责物理分块的生成、生命周期管理以及句柄分配，处理器本身不具备生成 PBlock 的权限。
*   **PBlock 类型**: `Dialogue` (对话), `File` (文件), `Stream` (流)。
*   **映射映射特征**:
    *   **结构化 PBlock**：如代码库、书籍章节。此类物理块映射后通常生成 **Consolidated Page**。
    *   **非结构化 PBlock**：如纯文本流。退化为固定尺寸分块，映射后生成 **Original Page**。

### 5.1 逻辑页 (Logical Pages) — LAS 层
代表**逻辑地址空间 (Logical Address Space, LAS)**。由 PBlock 经 Worker **即时映射 (Mapped)** 后生成。

#### 5.1.1 原始页 (Original Page)
最小逻辑单元（叶子节点）。
*   **清单 (Manifest)**:
    *   `id`: Short Hash。
    *   `origin`: `History | Storage`。
    *   `depth`: **逻辑深度 (Integer)**。
    *   `timestamp`: **逻辑定标 (ISO-8601)**。每一页必须携带绝对时间原点，用于时序排序与陈旧度判定。
    *   `keywords`: **语义关键词 (Optional)**。作为海选匹配的高维索引键。
    *   `summary`: **逻辑脉络提炼**。要求：**语义熵压缩**。严禁文学化描述，必须保留核心推论、关键变量及因果链条，作为 Router 检索的唯一依据。
    *   `content`: **高分辨率证据块**。要求：**物理保真**。完整呈现原始对话或数据，不进行任何非受控的语义删减，作为 Worker 最终推导的逻辑基石。

#### 5.1.2 综述页 (Consolidated Page)
作为**逻辑索引 (Logic Index)** 的容器节点。支持 `Unpacked` 变焦。
*   **清单 (Manifest)**:
    *   `id`: Short Hash。
    *   `depth`: **逻辑深度 (Integer)**。
    *   `timestamp`: **逻辑定标 (ISO-8601)**。
    *   `keywords`: **共识关键词**。代表容器内所有子页的语义交集。
    *   `source_ids`: 所含子页 ID 或子物理块地址。**支持递归嵌套**：子页 ID 可以指向另一个 Consolidated Page，形成 **逻辑树 (Logic Tree)**。
    *   `summary`: **共识性语义压缩**。由 Consolidator 对多个子页面（无论 Original 还是 Consolidated）执行逻辑归并后生成，代表该容器的“整理意志”。

### 5.3 页索引管理系统 (Page Index Management System)

为了支撑海量 Page 的秒级检索与逻辑变焦，PCP 维护了一个轻量级的**索引管理系统**：

*   **唯一寻址**: 每一个 Page（OP/CP）在索引中拥有全局唯一的 `Short Hash ID`。
*   **状态维护**: 索引实时跟踪 Page 的**热度**、**陈旧度**以及**当前激活状态**（是否已注入上下文）。
*   **并行主题拓扑**: 当多个主题并行推导时，索引通过 `Topic ID` 划分逻辑空间，确保 Router 在检索时能够快速执行隔离分压。


## VI. 意图驱动生命周期 (Intent-Driven Lifecycle)

PCP 采用以**意图 (Intent)** 为核心的寻址推演循环。每轮交互执行一次完整的地址扩散与物化过程：

### 1. 意图锚定 (Intent Anchoring)
*   **职责**: 为寻址确定语义极点。
*   **操作**: 系统分析当前视界的 **Head**（系统指令/全局变量）与 **Tail**（最新用户输入），提取出驱动本轮寻址的 **意图重心 (Intent Focus)**。
*   **重构**: 若输入熵过低（如“继续”），由 **Host Scaffolding** 自动拼接前轮 Summary 执行确定性语义重构，严禁依赖模型发散。

### 2. 神经寻址 (Neural Addressing)
*   **职责**: 在 Unified Address Space 中定位 Page 与物理块 (PBlock)。

#### 2.1 逻辑页寻址 (Logical Page Addressing — LAS)
*   **机制**: 基于 **Page Index** 的二级语义匹配。
    - **海选 (Broad Semantic Match)**：Router 根据 Intent Focus 与 Page 的 **语义关键词 (Keywords)** 在索引中快速召回具备相关性潜力的 Logical Pages。
    - **精选 (Precision Selection)**：通过高维语义对齐，深度对比 Intent Focus 与 Page Summary，决定 Pages 的激活状态（Hot 直接注入 / Indexed 仅摘要）。

#### 2.2 物理块寻址 (Physical Block Addressing — PAS)
*   **机制**: 基于 **意图引导 (Intent-Driven)** 的初步语义检索。
    - **召回策略**: Router 以 Intent Focus 为检索键，在 PAS 空间中预检索潜在相关的 **PBlock 挂接点**。
    - **投机性物化 (Speculative Materialization)**: 为解决 PBlock 的“盲盒”感知问题，Host 驱动 **Consolidator** 对召回的 PBlock 执行即时扫描，根据 Intent Focus 产出 **草稿页 (Draft Page)**。
    - **草稿页 (Draft Page)**: 
        - **形态**: draft 态的 Original 或 Consolidated Page。
        - **清单**: 包含基本 `id`、`summary` 及 `keywords`。
        - **作用**: 让 Worker 在不物化全文的情况下感知识别物理块的逻辑分布。
    - **注入态**: Draft Pages 挂接至视界。**平权展示策略**：
        - 高相关度：直接以 `view="Detail"` 展示（投机性全量物化）。
        - 低相关度：以 `view="Summary"` 展示（索引预留）。
    - **状态**: 此时 PBlock 处于“可探测态”。

#### 2.3 记忆获取 (Memory Acquisition)
*   **职责**: 从外部 Memory 系统调取相关的历史逻辑实体。
*   **机制**: 并行关键词检索。
    - **动作**: Router 在进行 LAS/PAS 寻址的同时，提取 Intent Focus 中的语义关键词（Keywords），并发向外部 Memory 系统发起接口调用。
    - **物化**: Memory 返回的高相关性 Page（符合 PCP 标准报文）直接作为本轮推演的“逻辑背景”注入上下文，参与后续的执行过程。


### 3. 视界合成与注入 (Synthesis & XML Construction)
*   **职责**: 构建 Worker 的感知界面。
*   **操作**: 按照时序排列 Page，并执行**溢出治理 (Overflow Control)**。
*   **背景化**: 对远场或低相关内容执行“综述性遗忘”，合并为 `<Background_Context>` 以维持窗口信噪比。

### 4. 执行与探测映射 (Execution & Proactive Mapping)
*   **职责**: 任务执行、地址穿透与未知探索。
*   **逻辑解构 (LAS Logic)**: Worker 通过 `Consult` 指令对视界内已有的 Page（包含 Draft Pages）执行分辨率提升，由 Host 完成物化填充。
*   **物理探测 (PAS Probing)**: 当 Worker 感知到当前 LAS 无法闭合逻辑链，且 Draft Page 提示物理块内存在关键线索时，通过 `Explore` 指令进行主动探测。
*   **映射权 (Mapping Sovereignty)**: Worker 以 Intent Focus 或 Explicit Keywords 为“滤镜”读取并解构 PBlock，仅提取出高度相关的 Page 实体。
*   **递归扩散 (Recursive Diffusion)**: 新物化的 Page 可能产生新的物理引用。Router 执行 **反应式扩散 (Reactive Diffusion)**，触发新一轮寻址。

### 5. 代谢、固化与归档 (Metabolism, Solidification & GC)
*   **动作**: 实时监听话题转折（Topic Pivot）与资源压强。
*   **综述生成触发 (CP Generation Triggers)**:
    - **语义触发 (Topic Pivot)**：检测到逻辑推演进入新阶段或话题发生转折，对前序 `Original Pages` 执行归并。
    - **物理触发 (Threshold)**：当前活跃视界内的原始页积累超过设定阈值（压重），执行强制归并以释放 Token 空间。
*   **草稿固化 (Draft Solidification)**:
    - **分辨率过滤**: 在每轮交互结束时，评估 Draft Pages 的视图状态。
    - **转正逻辑**: 凡处于非 `Summary` 态（即 `Detail` 或 `Unpacked`）的草稿页，视为已产生实质逻辑贡献，自动转正为 LAS 正式 Page。
    - **形态转换**: 
        - `Draft CP (Detail)` -> 转正为 `Original Page`（逻辑终点达成）。
        - `Draft CP (Unpacked)` -> 转正为 `Consolidated Page`（逻辑拓扑采纳）。
    - **丢弃**: 其余处于 `Summary` 态的 Draft Pages 在本轮结束后自动抹除，不进入持久化。
*   **长期代谢与进化 (Evolution)**: 
    - **水平归并 (Horizontal Merge)**：针对远场、陈旧且**属于同一主题**的相邻综述页进行合并（De-fragmentation），合并后保持同级深度，原始页集合取并集。
    - **垂直分叉 (Vertical Branching)**：当旧有话题跨时间被再次唤醒并产生逻辑分叉时，在分支点之上建立“综述之综述”，将历史结论与当前推演通过**树形分叉**保留。
*   **架构形态**: 驱动地址空间从“扁平时间流”进化为 **“时间轴主干 + 逻辑树分支”**，确保大规模复杂逻辑的精准定位与递归寻址。

---

## VII. 级联检索与 4 级递归变焦 (Zooming Mechanics)

变焦机制是 PCP 如何在极长交互中保持“既看清森林，又看清每一棵树”的核心路径。

### 7.1 三大语意视图 (Semantic View States)

为了让模型直观感知识读深度与物理属性（原子级 vs 容器级），PCP 采用语意视图系统：

1.  **`view="Summary"` (摘要)**: 展示逻辑提炼，隐藏底层数据。
2.  **`view="Detail"` (详情)**: 展示页面的完整内容。
    *   **Original 节点 (原子)**: 此为逻辑终点，不可再解构。
    *   **Consolidated 节点 (容器)**: 此为逻辑支点，可进一步解构。
3.  **`view="Unpacked"` (解构)**: 
    *   **约束**: **仅适用于 `type="Consolidated"` 节点**。且必须满足“激活变焦”约束：其内部必须**至少有一个**子节点处于非 `Summary` 状态（即 `Detail` 或更深），否则应自动执行 `Shelve` 回退至 `Detail` 态以保持视界紧凑。
    *   **表现**: 移除综述全文，直接嵌套展示其内部包含的子页面 (`Node`)。

### 7.2 递归变焦路径映射

*   **Level 1: 全局感知 (Global)**: 视界内的根页面以 `Summary` 呈现，构建逻辑概览。
*   **Level 2: 节点下钻 (Node Penetration)**: `Consult(id)` 使目标进入 `Detail`。
    *   **Original**: 露出物化全文。
    *   **Consolidated**: 露出容器综述全文。
*   **Level 3: 子树解构 (Sub-tree Unpacking)**: 对已处于 `Detail` 的 **Consolidated** 节点（综述态）再次调用 `Consult`，使其进入 `Unpacked` 露出内部子页面的 `Summary`。
    *   **递归特性**: 由于子页面可以是新的 Consolidated 节点，此过程支持**无限递归变焦**，允许在多级逻辑树中进行纵向搜索。
*   **Level 4: 原子还原 (Atomic Restoration)**: 对树末端的 `Original` 页面调用 `Consult` 成 `Detail`，触达逻辑终点。

### 7.3 协议指令定义 (Protocol Instruction Specs)

| 指令 | 调用格式 | 触发条件 | 效果 |
| :--- | :--- | :--- | :--- |
| **Consult** | `Consult(reason, id)` | 现有逻辑实体的分辨率不足以支撑结论 | **逻辑升级**：`Summary -> Detail` 或 `Detail -> Unpacked`。对象为 LAS 已知 ID。 |
| **Explore** | `Explore(reason, handle, keywords)` | 需要从未知物理块中提取特定逻辑 | **物理物化**：从 PBlock 句柄中根据 keywords 过滤并生成新 Page。对象为 PAS 句柄。 |
| **Shelve** | `Shelve(reason, id)` | 当前细节节点不再相关 | **视图降级**：`Unpacked -> Detail` 或 `Detail -> Summary`。 |

### 7.4 级联折叠逻辑 (Cascading Shelve / Auto-Folding)

为了保持上下文的极致纯净，`Shelve` 操作具备**级联折叠**特性：
*   **原子级触发**：当一个处于 `Detail` 状态的原子 Page 被 `Shelve` 后，它立即回退为 `Summary`。
*   **容器级坍缩**：当一个处于 Level 3（综述解构状态）的综述页，其内部包含的所有子页面 ID 都被 Worker 成功 `Shelve`（折叠）后，该综述页节点必须**自动向上坍缩**，回退到 Level 2（综述全文摘要）状态。
*   **逻辑目标**：确保思维视界中只存在“被明确需要的细节”，不留任何逻辑冗余。

## VIII. 物理映射与搜索逻辑 (Physical Mapping & Search Logic)

本节定义 PBlock 如何转化为 Page，以及 `Explore` 指令背后的物理搜索细节。

### 8.1 关键词驱动的熵抑制 (Keyword-Driven Search)
*   **语义滤镜**: `Explore` 并非盲目加载物理全文。Host 根据 `keywords` 在 PBlock 内部执行语义检索（BM25 或向量），仅提取与关键词强相关的片段。
*   **熵抑制**: 低于相关性阈值的物理噪音被强制保留在“物理空间”中，不被物化。这确保了逻辑地址空间的极高信躁比。

### 8.2 结构敏感性物化 (Structural Awareness)
Host 根据 PBlock 的物理属性决定其物化形态：
*   **原子物理块** (如：文本日志、函数片段) -> 物化为 **Original Page**。
*   **结构化物理块** (如：代码仓库目录、复杂文档章节) -> 物化为 **Consolidated Page (Draft)**。
    *   **物理变焦**：对结构化 Draft 调用 `Consult` 会触发 **子物理块扫描**，产出下一级的草稿页，实现物理与逻辑在变焦路径上的统一。

### 8.3 递归摘要合成 (Recursive Synthesis)
当物化一个复杂的结构化 PBlock 时，Consolidator 执行**递归综述合成**：
1.  自下而上计算子物理块的关键摘要。
2.  各层级摘要逐级归并，最终产出 Root Consolidated Page 的 `summary` 和 `keywords`。
3.  该过程是高度**意图相关**的——Summary 的侧重点由当前的 `Intent Focus` 决定。

**参数说明**:
*   **reason**: 操作的逻辑原因。
*   **id / handle**: 操作目标的逻辑 ID 或物理句柄。
*   **keywords**: `Explore` 专用，定义物理探测的语义滤镜。


## IX. 技术语法与协议规范 (XML Spec)

### 9.1 为什么采用 XML 级联 (XML Synthesis vs. Semantic Gluing)

PCP 并不直接将文本进行物理堆叠（Plain Text Gluing），而是通过 **XML 有序合成** 构建一个具备结构化确定性的感知界面，其核心设计动机包括：

*   **结构化消歧 (Structural Disambiguation)**：在处理具备复杂元数据（如 ID、Timestamp、Page Type）的上下文时，XML 提供了显式的逻辑边界。这能有效防止 LLM 对“协议控制指令”与“对话正文内容”产生认知混淆，降低推理中的幻觉偏移。
*   **寻址总线与操作闭环 (Addressing Bus)**：XML 标签为 `Consult` 和 `Shelve` 指令提供了确定的寻址对象（ID）。这使得外部系统能够像操作 DOM 一样，对上下文执行精确的局部增删改，而无需重新刷新全量语境。
*   **解析的确定性 (Parsing Determinism)**：现代长文本模型对结构化标记展现出极佳的遵循性。将上下文视为一个“可寻址的数据库”而非“平铺的文本流”，有助于模型维持严密的逻辑溯源（Tracing）能力。

### 9.2 核心标签规范 (Tag Specifications)

*   **`<PagedContext>`**: 协议根容器，携带 `version` 版本号。
*   **`<Static_Registry>`**: 静态注册表。注入不随对话变动的全局常量及**运行时指令**。
    *   **`<ST-Node>`**: **状态节点**。用于存储不随对话变动的系统状态或全局参量。包含 `id` 和 `value`。
    *   **`<System_Instructions>`**: **核心指令注入**。告知 Worker 该协议的操作守则（Manual），明确 `Consult/Shelve` 的触发时机与逻辑目标。
*   **`<Query>`**: **用户当前输入**。系统基于此输入执行意图识别与级联匹配。
*   **`<Reasoning_Trace>`**: **推理过程记录**。由一系列 `<Step>` 组成，记录 Worker 在之前轮次中执行的所有变焦动作。
    *   **`<Step>`**: 具体的动作项。包含 `action` (动作名), `target` (目标 ID) 和 `reason` (逻辑动机)。
*   **`<Linear_Flow>`**: **线性记忆流**。Page 节点集合，代表当前的“视界内容”。
*   **`<Node>`**: 逻辑页容器。
    *   `id`: 唯一识别 Short Hash。
    *   `type`: **页面物理属性 (Original | Consolidated)**。
    *   `view`: **语意视图 (Summary | Detail | Unpacked)**。
    *   `depth`: **层级定标 (Integer)**。
    *   `origin`: **来源追踪 (History | Storage)**。
    *   `keywords`: **语义索引键 (Comma-separated string)**。
    *   `timestamp`: 逻辑发生的时间原点。

---

### 9.3 XML 抽象样板 (Abstract Boilerplate)

用于系统集成的标准生成模板：

```xml
<PagedContext version="0.1.0-alpha">
  <Static_Registry>
    <ST-Node id="CURRENT_TIME" value="YYYY-MM-DDTHH:mm:ss" />
    <System_Instructions>
      - Original: 原始证据/对话节点 (不可拆解)。
      - Consolidated: 逻辑综述节点 (可 Unpacked)。
      - view="Summary": 摘要；view="Detail": 全文；view="Unpacked": 展开容器内部。
      - Consult(reason, id): 升级视图获取详情/子项；Shelve(reason, id): 降级视图清理视界。
    </System_Instructions>
  </Static_Registry>

  <Query>...</Query>

  <Reasoning_Trace>
    <Step action="Consult" target="PageID" reason="寻找高层逻辑背后的原始证据支撑。" />
  </Reasoning_Trace>

  <Linear_Flow>
    <!-- 1. 原子节点摘要 (Original Summary) -->
    <Node id="f1a2b3c4" type="Original" view="Summary" depth="1" keywords="CS, Addressing" origin="History" timestamp="2026-02-14T10:00:00">
      <Summary>背景参考摘要...</Summary>
    </Node>

    <!-- 2. 原子资料详情 (Original Detail - 逻辑终点) -->
    <Node id="d4e5f6a1" type="Original" view="Detail" depth="2" origin="Storage" timestamp="2026-02-14T10:05:00">
      <Content>最底层的对话全文或硬件原始日志...</Content>
    </Node>

    <!-- 3. 容器节点摘要 (Consolidated Summary) -->
    <Node id="b2c3d4e5" type="Consolidated" view="Summary" depth="1" timestamp="2026-02-14T10:10:00">
      <Summary>由 10 个 OP 页面合成的初步共识摘要。</Summary>
    </Node>

    <!-- 4. 容器节点详情 (Consolidated Detail - 递归中转) -->
    <Node id="c3d4e5f6" type="Consolidated" view="Detail" depth="1" timestamp="2026-02-14T10:15:00">
      <Content>综述全文，包含了完整的逻辑推论。此处可以继续下钻。</Content>
    </Node>

    <!-- 5. 容器节点解构 (Consolidated Unpacked - 直接嵌套) -->
    <Node id="e5f6a7b8" type="Consolidated" view="Unpacked" depth="1" timestamp="2026-02-14T10:20:00">
       <Node id="a7b8c9d0" type="Original" view="Summary" depth="2" origin="Storage" timestamp="..." />
       <Node id="f9e8d7c6" type="Original" view="Summary" depth="2" origin="Storage" timestamp="..." />
    </Node>
  </Linear_Flow>
</PagedContext>
```

---

## X. 哨兵机制与逻辑坍缩 (Sentry & Logic Collapse)

### 10.1 哨兵监控 (Sentry Logic)
系统实时监控 Token 压强 $P_{token}$ 与输入熵。当检测到压强越限或巨量新文件输入时，哨兵强制系统从“对话模式”切换为“协议模式”。

### 10.2 冷启动逻辑坍缩 (Retroactive Transition)
在切换瞬间，系统对之前的线性对话历史进行一次**脱水处理**：
*   **回溯性分页**：将之前的线性对话历史模拟 PCP 逻辑执行分页。根据话题转折 (Topic Pivot)、Token 预算及逻辑密度，将其坍缩为一系列符合协议规范的 `Original/Consolidated Pages`，确保历史信息的逻辑延续性。
*   **物理清空**：清空物理缓存，建立正式的 XML 寻址总线。
*   **身份坍缩**：Worker 的身份正式由“聊天伙伴”坍缩为**“逻辑处理器”**。

---

## XI. 逻辑持久化与回写 (Logic Persistence & Write-back)

PCP 并不负责维护网状的冷知识库，但它作为“逻辑处理器”，必须提供一种机制将实时处理产生的“逻辑净值 (Logical Net Value)”沉淀回外部系统。

### 11.1 记忆系统接口规范 (Memory Interface Requirements)
PCP 不关心 Memory 系统的内部实现，但其作为外部“逻辑插件”，必须满足以下调用契约：
*   **页兼容性报文 (Page Message Compatibility)**：Memory 的返回内容必须符合 `Original/Consolidated Page` 的 XML 定义，包含核心的 `summary` 与 `keywords` 用于 Router 进一步对齐。
*   **关键词检索接口 (Keyword Interface)**：Memory 必须暴露一个支持按语义关键词进行 Recall 的接口。Router 在 Addressing 阶段会提取意图关键词并行调用此接口。
*   **即时注入角色 (Instant Input Role)**：记忆获取是寻址流的一部分。检索到的 Pages 被视为当前 LAS 的有效延伸，享有与本地缓存同等的逻辑处理权重。

### 11.2 统一逻辑导出 (Unified Logic Export)
当任务流达到稳定逻辑终点或发生显著的“逻辑提取”行为（如生成高深度 Consolidated Page）时，宿主系统的 **Unified Export Manager** 被触发：
*   **导出资产**：PCP 抛出经由处理器加工后的 `summary`、`keywords` 以及建立的 `source_ids` 逻辑关联。
*   **黑盒沉淀 (External Sedimentation Policy)**：内容是否存入 Memory 以供后续会话实时寻址，还是沉淀至通用 RAG 作为非线性背景事实，**完全由接收方（Memory/RAG 系统）自主决定**。PCP 仅作为逻辑净值的生产者，不干涉外部存储系统的策略执行。

### 11.3 逻辑独立性 (State Isolation)
*   **非阻塞回写**：持久化过程对 PCP 实时任务流透明且异步。
*   **无副作用**：外部网状知识库（Memory/RAG）的更新不会实时导致 PCP 当前 Linear_Flow 的变化，除非 Worker 下一轮再次通过 Router 显式寻址到这些新沉淀的物理块。

---