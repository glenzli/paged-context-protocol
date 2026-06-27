# Paged-Context-Protocol (PCP) - v0.2.0-alpha

Paged-Context-Protocol (PCP) 是一种面向 LLM 应用的**统一逻辑寻址**与**分布式上下文治理**协议。它将碎片化的对话流与异构数据源（文件、流、仓库）映射为离散、可寻址的**逻辑页（Logical Pages）**，通过显式索引、按需下钻、后台整理和安全标注，在有限上下文窗口内提高信息召回、溯源和噪声控制能力。

## I. 核心目标：上下文虚拟化与逻辑地址空间 (Core Objectives)

PCP 将 LLM 视为可调用的逻辑处理组件，将外部上下文组织为可寻址、可压缩、可回溯的逻辑地址空间。其核心目标是把模型推理所需的信息边界从临时拼接的文本窗口中分离出来，使上下文管理成为可审计的系统过程。

### 1.1 线性逻辑流 vs. 外部系统 (Linear Logic vs. External)
PCP 定义了两类外部系统，并对其有不同的耦合要求：
*   **RAG 系统 (Generic Cold Storage)**：泛化的、非线性的外部数据库。PCP **完全不关心**其内部设计，仅将其视为 PBlock 的物理提供方。
*   **记忆系统 (Memory - PCP-native Logic Cache)**：PCP 的同源逻辑扩展缓存。Memory 存储经由 PCP 处理后的结构化逻辑，并尽量使用与运行时上下文相同的 Page Manifest、ID、`source_ids`、`trust` 与 Fetch 语义。这个强结构要求是有意设计：它使“当前上下文中的页”和“未注入上下文的历史页”可以在同一逻辑地址空间内被 Router 统一寻址、比较和下钻。非同源 Memory 可以通过 Adapter 包装为轻度兼容页；若声明为 PCP-native Memory，则应支持完整 Intent Prompt/Focus 查询、OP/CP 混合返回、按 ID/Source Span 拉取以及版本/溯源信息。
*   **PCP 协议 (Linear Logic Stream)**：当前任务的运行时线性流。它在任务 Timeline 上处理当前推理闭包，并利用 Memory 提供的历史逻辑资产辅助当前推演。

### 1.2 核心支柱
*   **角色重构 (LLM as Logic Processor)**：不将 LLM 视为事实存储器，而将其作为处理指令、推理关系和上下文状态的逻辑处理组件。其核心任务是消费 Page 间的拓扑关系和证据内容，而非被动承载全部文本。
*   **上下文虚拟化 (Context Virtualization)**：将历史、文档、代码等逻辑资产视为后台存储。物理窗口（Context Window）仅作为展示高分辨率细节的热缓存，用于在预算内维持可追溯的上下文视界。
*   **统一寻址逻辑 (Unified Addressing)**：通过 Pages 实现“内存与存储”的逻辑合一。无论是即时对话还是海量归档，均在同一套逻辑地址空间（LAS）内进行统一坐标定标与调度。
*   **按需分页 (Demand Paging)**：Worker 不应被动承载所有信息，而应通过 **Consult** 指令在地址空间中按需调取深层细节。
*   **证据约束推理 (Evidence-Bound Reasoning)**：当当前视界无法支撑结论时，Worker 应通过协议指令请求更多证据，而不是对地址空间外的信息进行无依据补全。


## II. 逻辑处理器模型 (Quad Processor Model)

系统基于四个核心角色的解耦协作运行，确保“治理”、“安全”与“执行”的分离：

1.  **寻址处理器 (Router-MMU)**: 
    *   **职责**: 逻辑坐标映射（Logical Mapping）。负责意图识别、逻辑页面索引、两阶段相关性匹配。
    *   **意图锚定**: 通过分析活跃上下文的"Head"（系统指令/全局上下文）与"Tail"（当前用户查询），提取当前**意图重心 (Intent Focus)**。
    *   **策略 CoT (Strategy CoT)**：在执行 Keywords/Summary 匹配前，Router 先读取当前活跃 Topic Tree Root CP 的 `schema` 字段（若存在），声明本轮寻址的「加权维度」（如 `code_evolution` Schema → 优先权重技术符号与接口名；`reasoning_chain` Schema → 优先权重因果连词与关键变量）。Router **不推断 Schema**，仅作为 Consolidator 下游产出的「被动消费者」，利用已有 Schema 执行差异化权重匹配。Schema 字段为空时退回通用权重匹配。
    *   **核心特征**: **模型辅助的逻辑相关性判断**。Router 可以利用模型对意图、摘要、关键词、时间和结构元数据进行综合判断，避免只依赖向量相似度或关键词命中。

2.  **执行处理器 (Worker-CPU)**: 
    *   **职责**: 任务执行与**即时映射 (JIT Mapping)**。Worker 负责在当前视界内执行任务，并在证据不足时提出下钻、探索或折叠请求。
    *   **映射决策**: Worker 负责将 Host 提供的**原始物理块 (Raw PBlocks)** 解构并映射为逻辑页面，Host 负责执行实际 I/O 与状态更新。
    *   **核心动作**: 动态确定物理数据的逻辑边界；触发“按需分页”指令；执行变焦与扩散算法。

3.  **整理处理器 (Consolidator-Background GC)**: 
    *   **职责**: 系统存储空间的后台维护（Memory Manager）。
    *   **核心动作**: 
        1.  **初始固化 (Freezing)**：监听话题状态与长度阈值。其“话题转折（Topic Pivot）”的判定是基于推理逻辑的突变，而非简单的语义距离。
        2.  **长效整合 (Merging)**：基于逻辑陈旧度执行“代谢合并”。
    *   **两阶段 Schema CoT (Two-Phase Schema CoT)**：Consolidator 在执行任何 `summary` 生成或合并前，必须先完成以下两阶段推断：
        -   **Phase 1 — Schema 识别**：声明当前逻辑流的 `Logic Schema`（合法值：`code_evolution | reasoning_chain | creative_world | tool_trace | mixed`），并识别该 Schema 下「高逻辑密度但低语义熵」的锚点类型——即在通用熵压缩视角下看似低频、但对后续推理具有关键约束作用的信息类别（如代码任务中的「方案排除路径」、推理链中的「单次出现的枢纽变量」、创作任务中的「世界状态 delta」）。
        -   **Phase 2 — 锚点导向压缩**：以 Phase 1 的 Schema 约束为滤镜执行压缩。**Schema 锚点的保留优先级高于通用语义熵压缩的精简冲动。** Phase 1 的识别结果写入所生成 CP 的 `schema` 元字段，供 Router 策略 CoT 使用。
    *   **固化产物信任约束 (Crystallized Trust Gate)**：Consolidator 生成 Consolidated Page 时，若所有输入源节点的 `trust` 均为 `system | history | audited`，则产物标记为 `trust="sealed"`；否则产物降级为 `trust="audited"`，下次召回时仍须经过运行时审查。

4.  **审计处理器 (Auditor-Security Gate)**:
    *   **职责**：截面①→②的强制安全关卡。在任何外部来源的内容（经由 `Explore` 物化的 PBlock 数据）进入 `<Linear_Flow>` 之前，Auditor 对其执行逐单元的二值语义判断（`PASS / BLOCK`）。
    *   **无执行权限约束**：Auditor **无权访问 `<Linear_Flow>` 的整体上下文，无权执行任何协议指令，无外部系统访问能力**。此约束降低了 Auditor 被注入后直接执行危险操作的风险。
    *   **分层防御原则**：外部内容必须先通过独立审查，再由 Worker 在带有 `trust` 标注的上下文中消费。Auditor 不是形式化安全边界，而是用于降低注入内容进入运行时视界的概率；应用层权限系统仍然负责限制最坏情况下的影响范围。
    *   **审计结果**：
        - `PASS`：内容加盖 `trust="audited"` 印记后由 Host 注入 Linear_Flow。
        - `BLOCK`：内容永久隔离，拒绝理由写入 `<Static_Registry>` 的 `<Security_Log>` 节点，本会话内不再召回。
    *   **部署独立性**：Auditor 与其他处理器物理隔离部署，可使用安全特化模型或轻量级判别模型，无需与 Worker/Consolidator 共享主模型实例。

### 2.4 PCP 处理器效能基准 (Processor Proficiency Baseline)

PCP 协议将逻辑寻址与状态控制拆分给多个角色。作为处理组件的 LLM 应满足以下工程要求：
*   **指令一致性 (Instruction Consistency)**：模型应具备稳定的结构化输出能力。协议定义的标记（如当前采用的 XML 标签）必须被遵守；语法截断、格式错位或结构破坏应被 Host 视为协议错误，并触发重试或中断。
*   **锚点保持压缩 (Anchor-Preserving Compression)**：执行整理 (Consolidation) 时，模型应在压缩文本的同时保留核心推演链条和物理标识符（如 ID、变量、参数）。不保留条件、否定路径或关键变量的摘要不应进入高信任索引层。
*   **缺口识别 (Gap Detection)**：当当前视界无法支撑结论时，Worker 应触发 `Consult` 或 `Explore` 请求更多证据。协议设计目标是鼓励下钻与溯源，而不是在证据不足时直接补全。

### 2.5 异构与并行部署策略

PCP 的四处理器架构支持 **异构多模型 + 并行部署 (Multi-LLM Parallel Strategy)**：

1.  **寻址层 (Router)**：
    *   **建议**：采用轻量级、高吞吐的高效模型。由于寻址属于“相对语义任务”，对吞吐量的要求远高于推理精度。
    *   **未来提案 (Proposal)**：随着低比特量化模型（如 BitNet b1.58）和端侧模型的发展，Router 是本地化部署的合适候选，可用于降低高频寻址成本。
        > [!NOTE]
        > 低比特模型能否稳定承担复杂路由任务仍需工程验证。若该路线效果不足，本地 4-bit 量化模型、专用 reranker 或远端轻量模型仍可作为 Router 的实现选项。
2.  **逻辑层 (Worker + Consolidator)**：
    *   **特性**：二者建议共享同一款高性能主模型（Main LLM）。由于 JIT Mapping 与 Consolidation 都涉及对数据进行结构化逻辑提取，使用一致的模型或一致的评测约束有助于降低语义漂移。
    *   **并行部署 (Parallel Instances)**：虽然模型一致，但物理上应通过并行实例部署。
        - **Worker (同步)**：响应用户主交互链路，确保即时性。
        - **Consolidator (异步)**：作为“后台 GC 进程”运行在独立并行实例中。它由 Host 监听 Token 压强或话题转折（Topic Pivot）后反应式触发，在后台完成索引合并与固化，从而避免后台维护任务阻塞用户主循环。

3.  **安全层 (Auditor)**：
    *   **建议**：Auditor 作为独立进程部署，与主模型链路完全隔离。可采用安全特化的判别模型（二值分类任务），也可使用经过安全微调的轻量模型，在低延迟条件下完成逐页审查。其响应延迟应低于 Explore 物化延迟，不成为系统瓶颈。

这种“高效寻址 + 一致的逻辑处理模型 + 异步维护并行化 + 独立安全门控”的部署方案，目标是在响应性能、上下文质量和信任隔离之间取得可控平衡。


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
    *   **指令翻译层**：接收 Worker 输出的 `Consult/Shelve/Purge` 等指令，并将其翻译为具体的物理存储读取（I/O）或索引状态修改动作（如施加负反馈权重）。
    *   **原子性保障**：确保每一轮指令执行的完整性，防止因局部读取失败导致逻辑状态机挂死。

4.  **意图脚手架 (Intent Scaffolding)**:
    *   **逻辑自愈**：在用户输入熵极低的情况下，自动提取前序 Page Summary 辅助意图重构，确保 Worker 始终在闭合的逻辑平面内工作。

5.  **统一导出管理器 (Unified Export Manager)**:
    *   **逻辑净值导出**：监听 Consolidator 的提取动作或任务结束信号。
    *   **职责边界**：PCP 不负责决策知识的最终去向。导出管理器仅通过 `Export_Logic_NetValue` 接口提供高质量的逻辑资产（如 Consolidated Page Summary、Keywords）。
    *   **无状态处理**：导出过程是异步且单向的，PCP 将逻辑资产交付给外部系统后即完成职责，由外部系统自主决定是存入 Memory 还是沉淀至 RAG。


## IV. 时间定标系统 (The Temporal Coordination System)

PCP 采用**时间轴 (Timeline)** 作为逻辑排序与关注度引导的核心规范：

*   **时间戳锚定**: 每一个逻辑页（Original/Consolidated）都必须携带明确的时间戳锚点。对于静态存储块，采用其**物理修改时间**或**逻辑加载序号**作为定标。
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
    *   `trust`: **信任级别**。合法值：`system | history | audited | sealed`。详见第 XII 章。
    *   `depth`: **逻辑深度 (Integer)**。
    *   `timestamp`: **逻辑定标 (ISO-8601)**。每一页必须携带明确时间原点，用于时序排序与陈旧度判定。
    *   `keywords`: **语义关键词 (Optional)**。作为海选匹配的高维索引键。
    *   `summary`: **逻辑脉络提炼**。要求：**锚点保持压缩**。摘要应避免文学化改写，必须保留核心推论、关键变量及因果链条，作为 Router 检索的主要依据；它不是最终证据本体。
    *   `anchors`: **关键锚点 (Optional)**。记录摘要中必须可回溯的变量、条件、否定路径、接口名、定理编号、文件路径等低频但关键的信息。
    *   `source_ref`: **物理来源引用**。指向 PBlock 句柄、文件路径、流 offset、对话轮次或外部 Memory Fetch 端点。
    *   `source_spans`: **来源范围 (Optional)**。用于记录行号、字节范围、token 范围、时间范围或结构化 AST 路径。数学、代码和审计场景中建议强制填写。
    *   `excerpt`: **中分辨率证据片段 (Optional)**。当全文过长或当前任务只需要局部证据时，Host 可只物化被选中的 source spans。
    *   `content`: **高分辨率证据块 (Optional / Materialized)**。仅在预算允许且任务需要时物化完整内容。Original Page 是逻辑原子，但不要求每次注入都携带完整物理全文；Host 必须保留可回拉的 `source_ref` / `source_spans`。
    *   `available_modes`: **可拉回分辨率集合 (Optional)**。声明该 Page 能够按需提供哪些证据层级，例如 `SummaryOnly,AnchoredSummary,Excerpt,Full`。这只是能力声明，不代表这些内容都已注入当前上下文。
*   **证据分辨率阶梯**：Original Page 的回拉不应被简化为“摘要 / 全文”二元状态，而应支持以下层级：
    - `SummaryOnly`：仅提供用于路由的锚点保持摘要，不作为最终证据。
    - `AnchoredSummary`：在摘要之外提供 `anchors`、`source_ref` 与可定位的 `source_spans`，适合先判断是否值得继续下钻。
    - `Excerpt`：返回与当前 Intent Focus 对齐的局部证据片段。
    - `Full`：返回完整物理内容，仅在局部片段不足或任务明确需要全文时使用。
*   **分辨率常驻原则**：分级是可用能力，不是默认载荷。当前 `<Linear_Flow>` 中只应放入当前所需的 `content_mode`；`available_modes` 与 `source_ref/source_spans` 留在索引或 Memory 中，用于后续按需升级。并非所有 Page 都必须支持所有分辨率，只有定义、命题、证明关键步、反例、接口变更、依赖枢纽等高价值节点才需要完整分级。

#### 5.1.2 综述页 (Consolidated Page)
作为**逻辑索引 (Logic Index)** 的容器节点。支持 `Unpacked` 变焦。
*   **清单 (Manifest)**:
    *   `id`: Short Hash。
    *   `trust`: **信任级别**。合法值：`system | history | audited | sealed`。Consolidator 根据输入源的 `trust` 值决定产物信任级别，规则见第 XII 章。
    *   `depth`: **逻辑深度 (Integer)**。
    *   `timestamp`: **逻辑定标 (ISO-8601)**。
    *   `keywords`: **共识关键词**。代表容器内所有子页的语义交集。
    *   `schema`: **逻辑结构类型标注 (Optional)**。由 Consolidator 两阶段 CoT 的 Phase 1 推断并写入。**仅 Root-level CP 必须填写**；子 CP 继承父级 Schema，无需重复标注。Router 在执行策略 CoT 时以此字段为权重参数。合法值：`code_evolution | reasoning_chain | creative_world | tool_trace | mixed`。
    *   `source_ids`: 所含子页 ID 或子物理块地址。**支持递归嵌套**：子页 ID 可以指向另一个 Consolidated Page，形成 **逻辑树 (Logic Tree)**。
    *   `summary`: **共识性语义压缩**。由 Consolidator 对多个子页面（无论 Original 还是 Consolidated）执行锚点导向压缩（Phase 2）后生成，代表该容器的"整理意志"。

### 5.3 页索引管理系统 (Page Index Management System)

为了支撑海量 Page 的秒级检索与逻辑变焦，PCP 维护了一个轻量级的**索引管理系统**：

*   **唯一寻址**: 每一个 Page（OP/CP）在索引中拥有全局唯一的 `Short Hash ID`。
*   **状态维护**: 索引实时跟踪 Page 的**热度**、**陈旧度**以及**当前激活状态**（是否已注入上下文）。
*   **主题拓扑 (Topic Topology)**: Consolidated Page 支持递归嵌套。在逻辑树中，最顶层的 Root Page 代表一个主题空间。系统通过管理不同逻辑树的 Root 节点，实现多主题并行推导与话题级隔离。
*   **Schema 作用域 (Schema Scoping)**：`schema` 字段的作用域与 **Topic Tree 的 Root CP** 绑定，而非全局会话状态。当 Consolidator 检测到 **Topic Pivot** 并创建新 CP 分支时，必须在新 Root CP 上执行独立的 Phase 1 Schema 识别，**不继承前序逻辑树的 Schema**。这确保跨话题的意图流转（如从代码讨论切换至架构设计）不会产生 Schema 干扰。Router 在读取 Schema 时，始终以**当前活跃 Topic Tree 的 Root CP** 为准。


## VI. 意图驱动生命周期 (Intent-Driven Lifecycle)

PCP 采用以**意图 (Intent)** 为核心的寻址推演循环。每轮交互执行一次完整的地址扩散与物化过程：

### 1. 意图锚定 (Intent Anchoring)
*   **职责**: 为寻址确定语义极点。
*   **操作**: 系统分析当前视界的 **Head**（系统指令/全局变量）与 **Tail**（最新用户输入），提取出驱动本轮寻址的 **意图重心 (Intent Focus)**。
*   **重构**: 若输入熵过低（如“继续”），由 **Host Scaffolding** 自动拼接前轮 Summary 执行确定性语义重构，避免仅依赖模型自由推断。

### 2. 模型辅助寻址 (Model-Assisted Addressing)
*   **职责**: 在 Unified Address Space 中定位 Page 与物理块 (PBlock)。

#### 2.1 逻辑页寻址 (Logical Page Addressing — LAS)
*   **机制**: 基于 **Page Index** 的二级语义匹配。
    - **海选 (Broad Semantic Match)**：Router 根据 Intent Focus 与 Page 的 **语义关键词 (Keywords)** 在索引中快速召回具备相关性潜力的 Logical Pages。
    - **精选 (Precision Selection)**：通过高维语义对齐，深度对比 Intent Focus 与 Page Summary，决定 Pages 的激活状态（Hot 直接注入 / Indexed 仅摘要）。
    - **分辨率规划 (Resolution Planning)**：Router 同时给出建议的 `desired_content_mode`。低相关或背景页通常为 `SummaryOnly`；需要确认锚点但暂不阅读证据时为 `AnchoredSummary`；强相关证据为 `Excerpt`；明确需要整体语境时才建议 `Full`。该建议不是最终物化命令，Host 仍根据 token 预算、`available_modes`、来源可访问性与安全策略决定实际注入的 `content_mode`。

#### 2.2 物理块寻址 (Physical Block Addressing — PAS)
*   **机制**: 基于 **意图引导 (Intent-Driven)** 的初步语义检索。
    - **召回策略**: Router 以 Intent Focus 为检索键，在 PAS 空间中预检索潜在相关的 **PBlock 挂接点**。
    - **投机性物化 (Speculative Materialization)**: 为解决 PBlock 的“盲盒”感知问题，Host 驱动 **Consolidator** 对召回的 PBlock 执行即时扫描，根据 Intent Focus 产出 **草稿页 (Draft Page)**。
    - **草稿页 (Draft Page)**: 
        - **形态**: draft 态的 Original 或 Consolidated Page。
        - **清单**: 包含基本 `id`、`summary` 及 `keywords`。
        - **作用**: 让 Worker 在不物化全文的情况下感知识别物理块的逻辑分布。
    - **注入态**: Draft Pages 挂接至视界。**平权展示策略**：
        - 高相关度：直接以 `view="Detail"` 展示（通常投机性 `Excerpt`，只有在任务明确需要且预算允许时才 `Full` 物化）。
        - 低相关度：以 `view="Summary"` 展示（索引预留）。
    - **状态**: 此时 PBlock 处于“可探测态”。

#### 2.3 记忆获取 (Memory Acquisition)
*   **职责**: 从外部 Memory 系统调取相关的历史逻辑实体。
*   **机制**: 完整意图查询与多级兼容。
    - **动作**: Router 在进行 LAS/PAS 寻址的同时，直接使用**完整的意图 Prompt/Focus** 向外部 Memory 系统发起查询（而非截取语义关键词）。
    - **分级物化与变焦支持**:
        - **轻度兼容 (Light Compatibility)**: 记忆系统提供单次搜索，Adapter 将返回内容包装为符合 PCP 定义的**原始页 (Original Page)**。这些 Page 至少应包含 `summary`、`source_ref`、`trust` 和可回拉的来源信息。
        - **同源兼容 (PCP-native Compatibility)**: 记忆系统原生返回支持 **原始页 / 综述页 (OP/CP)** 的混合嵌套结构，并保留与当前上下文一致的 ID、`source_ids`、`source_ref`、`source_spans`、`trust` 与版本信息。当返回物化的是综述页时，记忆系统必须提供按 ID 或 source span 查询的端点。若 Worker 对该记忆综述页执行 `Consult`，外部系统将直接返回被请求页面的摘要、局部 `excerpt` 或完整 `content`。这实现了从当前 Context 穿透至外部 Memory 空间的透明寻址体验。


### 3. 视界合成与注入 (Synthesis & XML Construction)
*   **职责**: 构建 Worker 的感知界面。
*   **操作**: 按照时序排列 Page，并执行**溢出治理 (Overflow Control)**。
*   **背景化**: 对远场或低相关内容执行“综述性遗忘”，合并为 `<Background_Context>` 以维持窗口信噪比。

### 4. 执行与探测映射 (Execution & Proactive Mapping)
*   **职责**: 任务执行、地址穿透与未知探索。
*   **逻辑解构 (LAS Logic)**: Worker 通过 `Consult` 指令对视界内已有的 Page（包含 Draft Pages）执行分辨率提升，由 Host 完成物化填充。
*   **物理探测 (PAS Probing)**: 当 Worker 感知到当前 LAS 无法闭合逻辑链，且 Draft Page 提示物理块内存在关键线索时，通过 `Explore` 指令进行主动探测。
*   **映射决策 (Mapping Decision)**: Worker 以 Intent Focus 或 Explicit Keywords 为过滤条件读取并解构 PBlock，仅提取出高度相关的 Page 实体。
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
2.  **`view="Detail"` (详情)**: 展示页面的已物化证据内容。
    *   **Original 节点 (原子)**: 此为逻辑终点，不可再解构；Detail 可展示 `excerpt` 或完整 `content`，由任务需求与预算决定。
    *   **Consolidated 节点 (容器)**: 此为逻辑支点，可进一步解构。
3.  **`view="Unpacked"` (解构)**: 
    *   **约束**: **仅适用于 `type="Consolidated"` 节点**。且必须满足“激活变焦”约束：其内部必须**至少有一个**子节点处于非 `Summary` 状态（即 `Detail` 或更深），否则应自动执行 `Shelve` 回退至 `Detail` 态以保持视界紧凑。
    *   **表现**: 移除综述全文，直接嵌套展示其内部包含的子页面 (`Node`)。
*   **视图与载荷正交**：`view` 描述 Worker 看到的逻辑形态，`content_mode` 描述当前注入的证据分辨率。`view="Detail"` 不等于 `content_mode="Full"`；一个 Detail 节点可以只携带 `AnchoredSummary` 或 `Excerpt`。

### 7.2 递归变焦路径映射

*   **Level 1: 全局感知 (Global)**: 视界内的根页面以 `Summary` 呈现，构建逻辑概览。
*   **Level 2: 节点下钻 (Node Penetration)**: `Consult(reason, id, target_view="Detail")` 使目标进入 `Detail`。
    *   **Original**: 露出相关 `excerpt` 或完整 `content`。
    *   **Consolidated**: 露出容器综述全文。
*   **Level 3: 子树解构 (Sub-tree Unpacking)**: 对已处于 `Detail` 的 **Consolidated** 节点（综述态）再次调用 `Consult(reason, id, target_view="Unpacked")`，使其进入 `Unpacked` 露出内部子页面的 `Summary`。
    *   **递归特性**: 由于子页面可以是新的 Consolidated 节点，此过程支持多级递归变焦，允许在逻辑树中进行纵向搜索。
*   **Level 4: 原子还原 (Atomic Restoration)**: 对树末端的 `Original` 页面调用 `Consult` 成 `Detail`，触达逻辑终点。Host 可先返回相关 `excerpt`，在 Worker 继续请求或证据不足时再升级为完整 `content`。

### 7.2.1 证据分辨率责任划分 (Evidence Resolution Responsibilities)

*   **Consolidator**：负责生产可路由的 `summary`、关键 `anchors`、候选 `source_spans`，并判断 Page 是否值得声明更高的 `available_modes`。它不应为低价值正文默认生成全套分级载荷。
*   **Router**：负责在召回时提出 `desired_content_mode`，用于排序和初始注入规划。
*   **Host / Context Manager**：负责最终物化决策。Host 根据 token 预算、`available_modes`、来源可访问性、Auditor 结果和当前窗口压强决定实际 `content_mode`，并可返回低于 Router/Worker 请求的分辨率。
*   **Worker**：只负责判断当前证据是否足以支撑推理。若不足，Worker 通过 `Consult` 请求更高分辨率或更精确的 `span_hint`。

### 7.3 协议指令定义 (Protocol Instruction Specs)

| 指令 | 调用格式 | 触发条件 | 效果 |
| :--- | :--- | :--- | :--- |
| **Consult** | `Consult(reason, id, target_view?, content_mode?, span_hint?)` | 现有逻辑实体的视图或证据分辨率不足以支撑结论 | **逻辑/证据升级**：`Summary -> Detail`、`Detail -> Unpacked`，或在同一视图内提升 `content_mode`。对象为 LAS 已知 ID；`content_mode` 不应超过该 Page 的 `available_modes`。 |
| **Explore** | `Explore(reason, handle, keywords)` | 需要从未知物理块中提取特定逻辑 | **物理物化**：从 PBlock 句柄中根据 keywords 过滤并生成新 Page。对象为 PAS 句柄。 |
| **Shelve** | `Shelve(reason, id)` | 当前细节节点信息已吸收或暂时不需要 | **视图降级**：`Unpacked -> Detail` 或 `Detail -> Summary`。 |
| **Purge** | `Purge(reason, id)` | 当前节点存在内容误判、与意图彻底无关或属废弃冗余 | **物理剔除与免疫**：将节点从 `<Linear_Flow>` 彻底删除，并在当前话题检索层对该 ID 施加强负反馈权重，防止后续重复召回。 |

### 7.4 级联折叠逻辑 (Cascading Shelve / Auto-Folding)

为了控制上下文规模，`Shelve` 操作具备**级联折叠**特性：
*   **原子级触发**：当一个处于 `Detail` 状态的原子 Page 被 `Shelve` 后，它立即回退为 `Summary`。
*   **容器级坍缩**：当一个处于 Level 3（综述解构状态）的综述页，其内部包含的所有子页面 ID 都被 Worker 成功 `Shelve`（折叠）后，该综述页节点必须**自动向上坍缩**，回退到 Level 2（综述全文摘要）状态。
*   **逻辑目标**：确保思维视界中只存在“被明确需要的细节”，不留任何逻辑冗余。

### 7.5 逻辑驱逐与负反馈机制 (Logical Eviction & Negative Feedback)

*   **视界清理**：`Purge` 指令强制要求系统从当前的 `<Linear_Flow>` 列表中完全抹除对应的 `<Node>`，释放宝贵的 Context Token 并不留任何逻辑杂音。
*   **检索隔离 (Negative Feedback)**：被 Worker `Purge` 掉的节点 ID，Host 将在当前的意图/话题流内对其施加负权重。这可以降低 Router 在后续检索循环中因错误相关性判断而重复召回该节点的概率。
*   **推理防错寻迹**：执行 `Purge` 的动作本身（带上极短的 reason）将以极低的 Token 成本沉淀在 `<Reasoning_Trace>` 中（例如：已检查文档 A，系旧版设定，已清除），作为模型在漫长推演中的认知记忆防错。

## VIII. 物理映射与搜索逻辑 (Physical Mapping & Search Logic)

本节定义 PBlock 如何转化为 Page，以及 `Explore` 指令背后的物理搜索细节。

### 8.1 关键词驱动的熵抑制 (Keyword-Driven Search)
*   **语义滤镜**: `Explore` 并非盲目加载物理全文。Host 根据 `keywords` 在 PBlock 内部执行语义检索（BM25 或向量），仅提取与关键词强相关的片段。
*   **噪声控制**: 低于相关性阈值的物理噪音保留在物理空间中，不被物化。这有助于维持逻辑地址空间的信噪比。

### 8.2 结构敏感性物化 (Structural Awareness)
Host 根据 PBlock 的物理属性决定其物化形态：
*   **原子物理块** (如：文本日志、函数片段) -> 物化为 **Original Page**。
*   **结构化物理块** (如：代码仓库目录、复杂文档章节) -> 物化为 **Consolidated Page (Draft)**。
    *   **结构下钻**：对结构化 Draft 调用 `Consult` 会触发 **子物理块扫描**，产出下一级的草稿页，使物理结构与逻辑变焦路径保持一致。

### 8.3 递归摘要合成 (Recursive Synthesis)
当物化一个复杂的结构化 PBlock 时，Consolidator 执行**递归综述合成**：
1.  自下而上计算子物理块的关键摘要。
2.  各层级摘要逐级归并，最终产出 Root Consolidated Page 的 `summary`、`keywords` 和 `schema`。
3.  该过程是高度**意图相关**的——Summary 的侧重点由当前的 `Intent Focus` 决定。
4.  在执行归并前，Consolidator 必须先完成 **Phase 1 Schema 识别**（即使是 Draft 物化阶段亦不例外）。**Schema 锚点的保留优先级高于通用语义熵的精简冲动**——对于在通用视角下看似冗余但在该 Schema 中具有关键逻辑地位的信息，必须在压缩后保持可追溯性。

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
    *   **`<System_Instructions>`**: **核心指令注入**。告知 Worker 该协议的操作守则（Manual），明确 `Consult/Shelve/Purge` 的触发时机与逻辑目标。
*   **`<Query>`**: **用户当前输入**。系统基于此输入执行意图识别与级联匹配。
*   **`<Reasoning_Trace>`**: **推理过程记录**。由一系列 `<Step>` 组成，记录 Worker 在之前轮次中执行的所有变焦动作。
    *   **`<Step>`**: 具体的动作项。包含 `action` (动作名), `target` (目标 ID) 和 `reason` (逻辑动机)。
*   **`<Linear_Flow>`**: **线性记忆流**。Page 节点集合，代表当前的“视界内容”。
*   **`<Node>`**: 逻辑页容器。
    *   `id`: 唯一识别 Short Hash。
    *   `type`: **页面物理属性 (Original | Consolidated)**。
    *   `trust`: **信任级别 (system | history | audited | sealed)**。决定本节点在 Linear_Flow 中的语义权威性，详见第 XII 章。
    *   `view`: **语意视图 (Summary | Detail | Unpacked)**。
    *   `depth`: **层级定标 (Integer)**。
    *   `keywords`: **语义索引键 (Comma-separated string)**。
    *   `timestamp`: 逻辑发生的时间原点。
    *   `schema`: **逻辑结构类型 (Optional, Root-level Consolidated only)**。由 Consolidator Phase 1 写入，Router 策略 CoT 读取。合法值：`code_evolution | reasoning_chain | creative_world | tool_trace | mixed`。
    *   `source_ref`: **来源引用 (Optional)**。指向可回拉的 PBlock、文件、流、对话轮次或外部 Memory Fetch 端点。
    *   `source_spans`: **来源范围 (Optional)**。以紧凑字符串或结构化路径记录行号、字节范围、token 范围、时间范围或 AST 路径，用于按需拉回局部证据。
    *   `content_mode`: **当前注入证据分辨率 (Optional)**。合法值：`SummaryOnly | AnchoredSummary | Excerpt | Full`。用于说明当前节点实际携带的是摘要、锚点化摘要、局部证据片段还是完整内容。
    *   `available_modes`: **可拉回证据分辨率 (Optional)**。合法值同 `content_mode`，可逗号分隔；用于声明该节点后续可通过 `Consult`/`Fetch` 升级到哪些分辨率。该字段不代表这些内容已经注入当前上下文。

---

### 9.3 XML 抽象样板 (Abstract Boilerplate)

用于系统集成的标准生成模板：

```xml
<PagedContext version="0.2.0-alpha">
  <Static_Registry>
    <ST-Node id="CURRENT_TIME" value="YYYY-MM-DDTHH:mm:ss" />
    <System_Instructions>
      - Original: 原始证据/对话节点 (不可拆解)。
      - Consolidated: 逻辑综述节点 (可 Unpacked)。
      - view="Summary": 摘要；view="Detail": 已物化证据详情；view="Unpacked": 展开容器内部。
      - Consult(reason, id, target_view?, content_mode?, span_hint?): 升级视图或证据分辨率；Shelve(reason, id): 降级视图清理视界；Purge(reason, id): 彻底剔除无关节点。
    </System_Instructions>
  </Static_Registry>

  <Query>...</Query>

  <Reasoning_Trace>
    <Step action="Consult" target="PageID" reason="寻找高层逻辑背后的原始证据支撑。" />
  </Reasoning_Trace>

  <Linear_Flow>
    <!-- 1. 系统级/历史节点摘要 (trust=system or trust=history) -->
    <Node id="f1a2b3c4" type="Original" view="Summary" depth="1" keywords="CS, Addressing" trust="history" timestamp="2026-02-14T10:00:00">
      <Summary>背景参考摘要...</Summary>
    </Node>

    <!-- 2. 历史推理详情节点 (trust=history, 逻辑终点) -->
    <Node id="d4e5f6a1" type="Original" view="Detail" depth="2" trust="audited" timestamp="2026-02-14T10:05:00" source_ref="pblock://log/a7" source_spans="L18-L35" content_mode="Excerpt" available_modes="SummaryOnly,AnchoredSummary,Excerpt,Full">
      <Excerpt>从原始日志中按 source span 选出的相关证据片段...</Excerpt>
    </Node>

    <!-- 3. 根级固化综述，携带 schema 字段 (trust=sealed, Root Consolidated) -->
    <Node id="b2c3d4e5" type="Consolidated" trust="sealed" view="Summary" depth="1" schema="code_evolution" timestamp="2026-02-14T10:10:00">
      <Summary>由 10 个 OP 页面合成的初步共识摘要；Schema 锚点保留：函数接口变更 delta 及方案排除路径。</Summary>
    </Node>

    <!-- 4. 容器节点详情 (Consolidated Detail - 递归中转) -->
    <Node id="c3d4e5f6" type="Consolidated" trust="sealed" view="Detail" depth="1" timestamp="2026-02-14T10:15:00">
      <Content>综述全文，包含了完整的逻辑推论。此处可以继续下钻。</Content>
    </Node>

    <!-- 5. 容器节点解构 (Consolidated Unpacked - 直接嵌套) -->
    <Node id="e5f6a7b8" type="Consolidated" trust="sealed" view="Unpacked" depth="1" timestamp="2026-02-14T10:20:00">
       <Node id="a7b8c9d0" type="Original" view="Summary" depth="2" trust="audited" timestamp="..." />
       <Node id="f9e8d7c6" type="Original" view="Summary" depth="2" trust="audited" timestamp="..." />
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
PCP 不关心 Memory 系统的内部实现，但如果 Memory 被声明为 PCP-native 同源逻辑缓存，则必须满足以下调用契约。非同源系统可以通过 Adapter 以轻度兼容方式接入。

> PCP-native Memory 的独立 Profile 见 [memory/SPEC.md](memory/SPEC.md)。本节只定义核心协议约束。

*   **页兼容性分级 (Page Compatibility Tier)**：
    - **轻度兼容**：返回内容由 Adapter 包装为 `Original Page`，至少包含 `summary`、`source_ref`、`trust` 和可回拉的来源信息。适用于普通搜索、文件检索或外部 RAG。
    - **同源兼容**：返回内容原生支持 `Original/Consolidated Page` 的任意层级混合结构，保留稳定 ID、`source_ids`、`source_ref`、`source_spans`、`trust`、版本和来源链。适用于希望与当前上下文抹平寻址差异的 PCP-native Memory。
*   **身份与驻留状态解耦 (Identity / Residency Decoupling)**：同源 Memory 中的 Page 身份由 `id`、`source_ids`、`source_ref`、版本与溯源链共同确定；该 Page 当前是否注入 `<Linear_Flow>` 只是 Host 维护的驻留状态（如 `in_context`、`indexed`、`external_memory`），不改变其逻辑身份。Router 与 Worker 因此可以用同一套 LAS 语义处理“当前上下文页”和“未注入的历史页”。
*   **查询与拉取接口 (Query & Fetch Interfaces)**：
    - **Query (意图查询)**：必须暴露支持**完整意图 Prompt** 查询的入口。Router 会直接传递其锚定的 Intent Focus 发起宽泛召回。
    - **Fetch (按需拉取 - 同源兼容必需)**：当外部引入了 Consolidated Page 或延迟物化的 Original Page 之后，系统必须提供按 `Short Hash ID`、`source_ref` 或 `source_spans` 直接拉取目标页面的接口，以响应 Worker 触发的 `Consult(id, content_mode?, span_hint?)` 从记忆中主动变焦的指令。Fetch 应支持返回 `SummaryOnly`、`AnchoredSummary`、`Excerpt` 或 `Full` 分辨率，并在返回节点中标明实际 `content_mode` 与可继续升级的 `available_modes`。若请求分辨率不可用或超过预算，Host/Memory 应返回不超过请求的最高可用分辨率并给出降级原因。
*   **即时注入角色 (Instant Input Role)**：记忆获取是寻址流的一部分。检索到的 Pages 被视为当前 LAS 的有效延伸，享有与本地感知态同等的逻辑处理权重。

### 11.2 统一逻辑导出 (Unified Logic Export)
当任务流达到稳定逻辑终点或发生显著的“逻辑提取”行为（如生成高深度 Consolidated Page）时，宿主系统的 **Unified Export Manager** 被触发：
*   **导出资产**：PCP 抛出经由处理器加工后的 `summary`、`keywords` 以及建立的 `source_ids` 逻辑关联。
*   **黑盒沉淀 (External Sedimentation Policy)**：内容是否存入 Memory 以供后续会话实时寻址，还是沉淀至通用 RAG 作为非线性背景事实，**完全由接收方（Memory/RAG 系统）自主决定**。PCP 仅作为逻辑净值的生产者，不干涉外部存储系统的策略执行。

### 11.3 逻辑独立性 (State Isolation)
*   **非阻塞回写**：持久化过程对 PCP 实时任务流透明且异步。
*   **无副作用**：外部网状知识库（Memory/RAG）的更新不会实时导致 PCP 当前 Linear_Flow 的变化，除非 Worker 下一轮再次通过 Router 显式寻址到这些新沉淀的物理块。

---

---

## XII. 安全模型 (Security Model)

### 12.1 核心前提与威胁截面定位

AI-Native 系统的安全前提不同于传统工程：上下文窗口同时承载指令、数据、工具结果和外部内容。模型可能把外部数据中的指令性文本误当成应执行的要求，因此不能依赖传统意义上的代码/数据物理边界。

PCP 作为上下文组装与治理协议，直接覆盖 AI-Native 执行管道的以下风险截面：

```
[外部世界 — 不可信]
       ↓ ── 截面①  摄入口：Explore 从 PBlock 摄入外部数据
[原始 PBlock 数据]
       ↓ ── 截面②  组装口：Synthesis Controller 将 Page 组装为 XML 报文
[<Linear_Flow> 上下文]
       ↓ ── 截面③  推理口：Worker 消费 Linear_Flow 产出 Intent
[Intent 输出]
       ↓ ── 截面⑤  落盘口：Consolidator 产物写入 Memory
[长期存储 — sealed 固化层]
```

### 12.2 trust 类型系统

`trust` 是 PCP 协议的安全类型系统的核心字段，取代先前的 `origin` 字段。**进入 `<Linear_Flow>` 的节点的 `trust` 值必须是且仅是以下四者之一**：

| `trust` 值 | 含义 | 来源 | 进入方式 |
| :--- | :--- | :--- | :--- |
| `system` | 系统级常量，最高信任 | `<Static_Registry>` / System Instructions | Host 直接注入 |
| `history` | 当前对话的 Worker 推理产物 | 对话历史 | 由 Host 标记注入 |
| `audited` | 来自外部 PBlock，已通过 Auditor 审查 | `Explore` 物化后 Auditor PASS | Auditor 门控后注入 |
| `sealed` | Consolidator 对可信内容的压缩固化产物 | Consolidator 处理纯可信输入后生成 | Consolidator 产出 |

> **协议级不变式**：任何 `trust` 值不在以上四者内的节点，**禁止进入 `<Linear_Flow>`**。Host 负责在注入前完成此检查。

### 12.3 Auditor 关卡规范（截面①→②）

Auditor 是 PCP 在截面①→②处的强制安全门控，其核心规范如下：

1. **触发时机**：Worker 发出 `Explore` 指令后，Host 物化 PBlock 数据为 Draft Page，此时**必须经过 Auditor 审查后方可注入 Linear_Flow**，不可绕过。
2. **Auditor 的上下文隔离**：Auditor 只接收**待审查的单个 Draft Page 内容**，不接收 `<Linear_Flow>` 的其他节点，不接收系统指令，无外部系统调用能力。
3. **输出格式**：`PASS` 或 `BLOCK`，附 `reason` 字段。
4. **PASS 处理**：Host 将 Draft Page 的 `trust` 字段设置为 `"audited"` 后注入 Linear_Flow。
5. **BLOCK 处理**：Draft Page 永久丢弃；Host 将 `<Block id="..." reason="..." timestamp="..."/>` 记录写入 `<Static_Registry>` 内的 `<Security_Log>` 节点；对应 PBlock 句柄在当前话题检索层施加强负面权重。
6. **分层防御效果**：Auditor 无执行权限这一约束确保了——即使 Auditor 被欺骗（输出 PASS），Auditor 本身也无法直接执行危险操作。攻击者仍可能构造同时通过审查且影响 Worker 的内容，因此 Auditor 只能降低风险，不能替代应用层权限控制和运行时策略。

### 12.4 Consolidator 固化约束（截面⑤）

Consolidated Page 写入 Memory 后作为历史推理共识被召回，享有更高的语义权威性。为防止污染内容被“蒸馏”进固化层，协议规定：

*   **`trust="sealed"` 的充要条件**：Consolidator 处理的所有输入节点的 `trust` 值均属于 `{system, history, audited, sealed}`，即全部来自已清洁的可信链路。
*   **降级规则**：若存在任意输入节点不满足上述条件，产物 `trust` 降级为 `"audited"`，表明该 CP 在未来召回时不可作为固化指令无条件执行，需继续受语义审查约束。
*   **意义**：切断“攻击成功 → 语义蒸馏 → 固化持久化 → 后门永久生效”的攻击链。

### 12.5 Linear_Flow 信任不变式（协议保证）

PCP 协议给出以下可在协议层验证的安全不变式：

> **[不变式 S1]** `<Linear_Flow>` 内所有节点的 `trust` 值必须属于 `{system, history, audited, sealed}`。任何 `trust` 值缺失或越界的节点进入 `<Linear_Flow>` 均视为协议违规。

> **[不变式 S2]** `trust="audited"` 的节点已通过 Auditor 审查，但仍属于**不可信数据**（非指令）。Worker 的 System_Instructions 明确禁止将 `audited` 节点的内容解读为协议指令；如 Worker 在 `audited` 内容中发现明显指令语义，必须立即 `Purge` 并在 `<Reasoning_Trace>` 中记录。

> **[不变式 S3]** `trust="sealed"` 节点的产生，要求其全部输入来源已处于可信链路（`system | history | audited | sealed`）。Consolidator 不得绕过此约束生成 `sealed` 产物。

### 12.6 PCP 在防御体系中的分层定位

PCP 的安全机制属于**软防御（语义层，概率性）**，负责大幅提高攻击成本、过滤绝大多数注入威胁。硬边界（应用层权限系统，确定性）负责限制最坏情况下的爆炸半径。两者互为补充，不可替代：

| 防御层次 | 实施者 | 防御性质 | 覆盖截面 |
| :--- | :--- | :--- | :--- |
| trust 类型系统 | PCP Host（协议级标注） | 确定性协议约束 | 截面①② |
| Auditor 语义沙箱 | Auditor 处理器 | 概率性（大幅提升攻击代价） | 截面①→② |
| Consolidator 固化约束 | Consolidator 处理器 | 概率性 + 确定性规则 | 截面⑤固化层 |
| 应用层权限封装 | AI-Native 应用自实现 | **形式化确定** | 截面④ |
