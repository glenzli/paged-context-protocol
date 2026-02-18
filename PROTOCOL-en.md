# Paged-Context-Protocol (PCP) - v0.1.0-alpha

Paged-Context-Protocol (PCP) is a low-level protocol focused on **Unified Logical Addressing** and **Distributed Context Governance** for LLMs. It treats the LLM as a **Flexible Execution Logic CPU**, mapping fragmented dialogue streams and massive heterogeneous data sources (files, streams, repositories) into discrete, addressable **Logical Pages**. It physically resolves context pollution and information overload in long-range interactions, empowering models to exercise **Logical Sovereignty** within an infinite address space.

## I. Core Philosophy: Flexible Compute & Logical Virtual Memory (LVM)

The PCP protocol treats the LLM as a **Flexible Execution Logic CPU** and defines this specification as the **Logical Virtual Memory (LVM)** protocol for that processor. Its core objective is to achieve deep decoupling between intelligent logic and its physical vehicle.

### 1.1 Linear Logic vs. External Systems
PCP defines two categories of external systems with different coupling requirements:
*   **RAG System (Generic Cold Storage)**: A generalized, non-linear external database. PCP is **entirely indifferent** to its internal design, treating it only as a physical provider of PBlocks.
*   **Memory System (Logic Extended Cache)**: The **Logic Extended Cache** of PCP. Memory is a critical "plug-in" that stores structured logic processed by PCP. PCP has **strong coupling requirements** for it: Memory must be able to provide high-relevance content matching the PCP page definition based on semantic keywords, serving as a constant input for the Router's intent matching.
*   **The PCP Protocol (Linear Logic Stream)**: The **intent-driven "Runtime Hot Stream"**. It handles logical closure on the task's Timeline, utilizing historical logic assets provided by Memory to assist current derivation.

### 1.2 Core Pillars
*   **Role Refactoring (LLM as CPU)**: No longer viewing the LLM as a "Memory-heavy Encyclopedia" but defining it as a **Logic Processor** focused on instruction execution and logical flow management. Its primary task is handling the topological relationships between Pages rather than mere text generation.
*   **Context Virtualization**: Treating all logical assets (history, documents, codebases) as "Backing Store." The physical context window serves only as a **"Hot Cache (L1/L2 Cache)"** for displaying high-resolution details, enabling "infinite-length" logical perception.
*   **Unified Addressing Logic**: Achieving the logical union of "Memory and Storage" through Pages. Whether it's real-time dialogue or massive archives, all are identified and scheduled within a unified Logical Address Space (LAS).
*   **Demand Paging**: The Worker should not passively carry all information but act as a **Memory Management Unit (MMU)**, autonomously retrieving deep details from the address space via the **Consult** command.
*   **Logical Sovereignty**: PCP grants the execution actor absolute authority over judgment. As a logic processor, the model is strictly prohibited from "semantic filling" (hallucinating) unknown information outside the address space; all logical gaps must be resolved through physical penetration (Zooming/Mapping).

## II. Trio Processor Model

The system operates based on the decoupled collaboration of three core roles, ensuring the separation of "governance" and "execution":

1.  **Addressing Processor (Router-MMU)**: 
    *   **Responsibility**: Logical coordinate mapping (Logical Mapping). Handles intent recognition, logical page indexing, and two-stage relevance matching.
    *   **Intent Anchoring**: Extracts the current **Intent Focus** by analyzing the "Head" (System Instructions/Global Context) and "Tail" (Current User Query) of the active context.
    *   **Core Characteristic**: **Neural Addressing over Numerical Retrieval**. The Router utilizes high-dimensional semantic space to evaluate logical correlations and generates **Addressing Instructions** to drive Mapping.

2.  **Execution Processor (Worker-CPU)**: 
    *   **Responsibilities**: Task execution and **JIT Mapping**. The execution processor is no longer a simple generator but is deconstructed as an execution unit with high-dimensional logical perception, managing logical inference and exercising mapping sovereignty.
    *   **Mapping Sovereignty**: As the exerciser of logical sovereignty, the Worker is responsible for deconstructing and mapping the **Raw PBlocks** provided by the Host into logical Pages.
    *   **Core Actions**: Dynamically determines the logical boundaries of physical data; triggers "Demand Paging"; executes zooming and diffusion algorithms.

3.  **Consolidation Processor (Consolidator-Background GC)**: 
    *   **Responsibility**: Physical maintenance of storage space (Memory Manager).
    *   **Core Action**: 
        1.  **Initial Freezing**: Monitors topic states and length thresholds. Its judgment of a "Topic Pivot" is based on inferential logic shifts rather than simple semantic distance.
        2.  **Metabolic Merging**: Performs "merging metabolism" based on logical staleness.

### 2.4 Processor Proficiency Baseline

The PCP protocol highly decouples logical addressing and state control, delegating them to the executor. Therefore, the LLM acting as a processor must meet the following "Logic Physics" baselines to ensure effective logical inference in complex environments:

*   **Instructional Consistency**: The model must possess extreme **Structural Output Robustness**. The markers defined by the protocol (such as the currently adopted XML tags) must be strictly followed as "physical boundaries." Any uncontrolled syntax truncation, format misalignment, or structural breach will be treated as a **Bus Fault**, forcing a protocol task interruption.
*   **Semantic Entropy Compression**: When performing Consolidation, the model must maintain "**Logical Anchor Distortion-Free**." It must preserve core deduction chains and physical identifiers (e.g., IDs, values, variables) while compressing text. "Literary summaries" are treated as **Payload Errors** in PCP.
*   **Proactive Pressure Sensing**: As a Worker, the model must possess **"Logic Vacuum Perception"** capabilities. When the current horizon cannot close the logic chain, it must precisely trigger the `Consult` instruction. Performing "semantic filling" (hallucination) under insufficient resolution is strictly prohibited. Proactive **Zooming** is prioritized over blind reasoning.

### 2.5 Heterogeneous & Parallel Deployment Strategy

The Trio Processor Model of PCP supports a **Dual-LLM Parallel Deployment Strategy**:

1.  **Addressing Layer (Router)**:
    *   **Recommendation**: Use a lightweight, high-throughput model. Since addressing is a "relative semantic task," throughput requirements outweigh deep reasoning precision.
    *   **Future Proposal**: With the maturation of **1.58-bit extreme quantization models** (e.g., BitNet b1.58), the Router is an ideal candidate for local deployment of such models, enabling near-zero-cost, high-frequency logical addressing collaboration.
2.  **Logic Layer (Worker + Consolidator)**:
    *   **Consistency Requirement**: Both **must share the same high-performance Main LLM**. Because JIT Mapping and Consolidation both involve "absolute logical summarization/extraction" of data, they must use the same logical metrics to prevent semantic drift.
    *   **Parallel Deployment (Parallel Instances)**: Although the model is consistent, they should be deployed via parallel physical instances.
        - **Worker (Synchronous)**: Responds to the main user interaction loop, ensuring immediacy.
        - **Consolidator (Asynchronous)**: Runs as a "background GC process" in a separate parallel instance. It is reactively triggered by the Host upon Token pressure or Topic Pivots, performing index merging and freezing in the background to prevent maintenance tasks from blocking the user's main cycle.

This deployment scheme—combining "Efficient Addressing + Logically Consistent Main Model + Asynchronous Maintenance Parallelization"—ensures system logical closure while achieving excellent responsive performance.

## III. The Host System Model

The Host system provides the "rigid" physical environment for protocol execution. As the infrastructure of PCP, it handles resource management and message synthesis, ensuring the physical safety and logical consistency of the protocol:

1.  **Index Engine (Index Manager)**:
    *   **Page Registration & ID Allocation**: Responsible for the globally unique `Short Hash` allocation of all pages within the Logical Address Space (LAS).
    *   **Heat Level & State Tracking**: Maintains the activation status (Hot/Indexed/Shelved) and logical weight of pages in the index.
    *   **PBlock Governance**: Fully manages the addressing handles for physical blocks and the cold/hot exchange of physical resources.

2.  **Synthesis Controller (XML Engine)**:
    *   **View Pipeline**: Materializes structured XML messages for injection into the Worker processor based on addressing results.
    *   **Overflow Governance**: The core safeguard logic. When the Worker triggers a `Consult` or `Mapping Pulse` that risks overflowing the physical context, the controller dynamically evaluates window pressure. It enforces "Dynamic Dehydration" on non-intent-focus pages—mandating a `view="Detail" -> "Summary"` degradation before injection.

3.  **Bus Mediator**:
    *   **Instruction Translation**: Receives directives like `Consult(id)` or `Shelve(id)` from the Worker and translates them into physical I/O (storage reads) or index state updates.
    *   **Atomicity Assurance**: Ensures the integrity of each interaction round, preventing logic state hangs due to partial I/O failures.

4.  **Intent Scaffolding**:
    *   **Logical Self-Healing**: In cases of extremely low input entropy, automatically extracts prefix Page Summaries to aid intent reconstruction, ensuring the Worker always operates within a closed logical plane.

5.  **Unified Export Manager**:
    *   **Logical Net Value Export**: Listens for the extraction actions of the Consolidator or task completion signals.
    *   **Responsibility Boundary**: PCP is not responsible for deciding the final destination of knowledge. The Export Manager only provides high-quality logical assets (such as Consolidated Page Summaries and Keywords) via the `Export_Logic_NetValue` interface.
    *   **Stateless Processing**: The export process is asynchronous and one-way. PCP's responsibility ends once the logical assets are delivered to the external system, which then independently decides whether to store them in Memory or sediment them into RAG.


## IV. The Temporal Coordination System

PCP uses a **Timeline** as the core specification for logical ordering and focus guidance:

*   **Timestamp Anchoring**: Every logical page (Original/Consolidated) must carry an absolute timestamp. For static storage blocks, **Physical Modification Time** or **Logical Load Sequence** is used as the anchor.
*   **Current Time Injection**: At the top of each interaction's context, `Current_Time` is explicitly injected.
*   **Temporal Awareness**: The Worker compares Page timestamps with `Current_Time` to determine logical order or data freshness.

## V. Logical Entities & Address Space

PCP maintains two layers of address space, interacting through the Worker's mapping behavior:

### 5.0 Physical Block (PBlock) — PAS Layer
*   **Responsibility**: Represents the physical address space (Physical Address Space, PAS). The **Host (System)** is responsible for the generation, lifecycle management, and handle allocation of physical blocks; the processor itself does not have the authority to create PBlocks.
*   **PBlock Types**: `Dialogue`, `File`, `Stream`.
*   **Mapping Characteristics**:
    *   **Structured PBlocks**: e.g., code repositories, book chapters. These physical blocks typically map to a **Consolidated Page**.
    *   **Unstructured PBlocks**: e.g., plain text streams. Degrade to fixed-size blocks, mapping to an **Original Page**.

### 5.1 Logical Pages — LAS Layer
Represents the **Logical Address Space (LAS)**. Generated through **JIT Mapping** of PBlocks by the Worker.

#### 5.1.1 Original Page
The smallest logical unit (leaf node).
*   **Manifest**:
    *   `id`: Short Hash identifier.
    *   `origin`: `History | Storage`.
    *   `depth`: **Logical Depth (Integer)**.
    *   `timestamp`: **Temporal Anchor (ISO-8601)**. Every page must carry an absolute time origin for sequence ordering and staleness judgment.
    *   `keywords`: **Semantic Keywords (Optional)**. Serves as high-dimensional index keys for broad match retrieval.
    *   `summary`: **Logical Essence extraction**. Requirement: **Semantic Entropy Compression**. Literary descriptions are prohibited; must preserve core deductions, key variables, and causal chains as the sole retrieval basis for the Router.
    *   `content`: **High-Resolution Evidence Block**. Requirement: **Physical Fidelity**. Presents the original dialogue or data in full without uncontrolled semantic truncation, serving as the logical bedrock for final derivation by the Worker.

#### 5.1.2 Consolidated Page
Serves as a container node for **Logic Indexing**. Supports `Unpacked` zooming.
*   **Manifest**:
    *   `id`: Short Hash identifier.
    *   `depth`: **Logical Depth (Integer)**.
    *   `timestamp`: **Temporal Anchor (ISO-8601)**.
    *   `keywords`: **Consensus Keywords**. Represents the semantic intersection of all child pages within the container.
    *   `source_ids`: Array of contained child Page IDs or sub-block addresses. **Supports Recursive Nesting**: Child IDs can point to another Consolidated Page, forming a **Logic Tree**.
    *   `summary`: **Consensus Semantic Compression**. Generated after the Consolidator performs logical merging of multiple sub-pages (whether Original or Consolidated), representing the "collective will" of the container.

### 5.3 Page Index Management System

To support second-level retrieval and logical zooming of massive Pages, PCP maintains a lightweight **index management system**:

*   **Unique Addressing**: Every Page (OP/CP) has a globally unique `Short Hash ID` in the index.
*   **State Maintenance**: The index tracks the **heat level**, **staleness**, and **current activation status** (whether it's injected into the context) of Pages in real-time.
*   **Parallel Topic Topology**: When multiple topics are reasoned in parallel, the index partitions logical space via `Topic ID`, ensuring the Router can quickly perform isolation and pressure relief during retrieval.


## VI. Intent-Driven Lifecycle

PCP employs an inference loop centered around **Intent**. Every interaction performs a complete addressing diffusion and materialization process:

### 1. Intent Anchoring
*   **Responsibility**: Defines the semantic pole for addressing.
*   **Action**: The system analyzes the **Head** (System instructions/Global variables) and **Tail** (Latest user input) of the current horizon to extract the **Intent Focus** driving this round.
*   **Reconstruction**: If input entropy is too low (e.g., "continue"), the **Host Scaffolding** automatically concatenates the previous round's Summary to perform deterministic semantic reconstruction.

### 2. Neural Addressing
*   **Responsibility**: Locates Pages and Physical Blocks (PBlock) within the Unified Address Space.

#### 2.1 Logical Page Addressing (LAS)
*   **Mechanism**: Two-stage semantic matching based on the **Page Index**.
    - **Broad Semantic Match**: The Router performs coarse-grained filtering in the index based on the Intent Focus and the Page's **Semantic Keywords**, recalling Logical Pages with relevance potential.
    - **Precision Selection**: Performs deep semantic alignment, comparing Intent Focus with Page Summary to determine Page activation states (**Hot** for direct injection / **Indexed** for summary only).

#### 2.2 Physical Block Addressing — PAS
*   **Mechanism**: Preliminary semantic retrieval based on **Intent-Driven** guidance.
    - **Recall Strategy**: The Router uses Intent Focus as a search key to pre-retrieve potentially relevant **PBlock attachment points** within the PAS space.
    - **Speculative Materialization**: To solve the "blind box" perception problem of PBlocks, the Host drives the **Consolidator** to perform an immediate scan of recalled PBlocks, generating **Draft Pages** based on the Intent Focus.
    - **Draft Page**: 
        - **Form**: Original or Consolidated Page in a `draft` state.
        - **Manifest**: Contains basic `id`, `summary`, and `keywords`.
        - **Role**: Allows the Worker to perceive the logical distribution of physical blocks without materializing the full content.
    - **Injection State**: Draft Pages are attached to the horizon. **Equalized Presentation Strategy**:
        - High Relevance: Displayed directly as `view="Detail"` (speculative full materialization).
        - Low Relevance: Displayed as `view="Summary"` (index reservation).
    - **Status**: At this stage, PBlocks are in a **"detectable"** state.

#### 2.3 Memory Acquisition
*   **Responsibility**: Retrieves relevant historical logic entities from the external Memory system.
*   **Mechanism**: Parallel keyword retrieval.
    - **Action**: While performing LAS/PAS addressing, the Router extracts semantic keywords from the Intent Focus and initiates concurrent interface calls to the external Memory system.
    - **Materialization**: High-relevance Pages returned by Memory (compliant with PCP standard messaging) are directly injected into the context as "logical background" for the current round, participating in subsequent execution.


### 3. Synthesis & XML Construction
*   **Responsibility**: Builds the Worker's perception interface.
*   **Action**: Order Pages chronologically and enforce **Overflow Governance**.
*   **Backgrounding**: Perform "summary-style forgetting" on far-field or low-relevance content, merging it into `<Background_Context>` to maintain window signal-to-noise ratio.

### 4. Execution & Proactive Mapping
*   **Responsibility**: Task execution, address penetration, and unknown exploration.
*   **Logical Deconstruction (LAS Logic)**: The Worker uses the `Consult` instruction on existing Pages (including Draft Pages) in the horizon to upgrade resolution, which is then fulfilled by the Host.
*   **Physical Probing (PAS Probing)**: When the Worker perceives that the current LAS cannot close the logic chain and Draft Pages suggest critical clues within a physical block, it performs proactive probing via the `Explore` instruction.
*   **Mapping Sovereignty**: The Worker reads and deconstructs PBlocks using the Intent Focus or Explicit Keywords as a "filter," extracting only highly relevant Page entities.
*   **Recursive Diffusion**: Newly materialized Pages may generate new physical references. The Router performs **Reactive Diffusion**, triggering a new round of addressing.

### 5. Metabolism, Solidification & GC
*   **Action**: Real-time monitoring for topic pivots and resource pressure.
*   **Consolidated Page (CP) Generation Triggers**:
    - **Semantic Trigger (Topic Pivot)**: When logical deduction enters a new phase or a shift in topic is detected, preceding `Original Pages` are merged.
    - **Physical Trigger (Threshold)**: When the accumulation of original pages in the active horizon exceeds a set threshold, a mandatory merge is performed to free up Token space.
*   **Draft Solidification**:
    - **Resolution Filter**: At the end of each interaction round, evaluate the view state of all Draft Pages.
    - **Solidification Logic**: Any Draft Page in a non-`Summary` state (i.e., `Detail` or `Unpacked`) is deemed to have made a substantial logical contribution and is automatically solidified as a formal LAS Page.
    - **Form Transformation**: 
        - `Draft CP (Detail)` -> Solidified as `Original Page` (Logical endpoint reached).
        - `Draft CP (Unpacked)` -> Solidified as `Consolidated Page` (Logical topology adopted).
    - **Discarding**: Any other Draft Pages remaining in the `Summary` state are discarded at the end of the round and do not enter persistence.
*   **Long-term Metabolism & Evolution**: 
    - **Horizontal Merge (CP Defragmentation)**: Performs merging for far-field, stale, and **same-topic** adjacent Consolidated Pages. The resulting node maintains the same depth level, with its source IDs being the union of the originals.
    - **Vertical Branching (Logic Tree Evolution)**: When an old topic is revisited across time and generates a new logical fork, a "CP of CPs" is established above the branch point, preserving historical conclusions and current deductions through **tree-like branching**.
*   **Architectural Topology**: Drives the address space to evolve from a "flat temporal stream" into a **"Temporal Backbone + Logical Tree Branches"** model, ensuring precise positioning and recursive addressing for large-scale complex logic.

---

## VII. Zooming Mechanics

Zooming is the core path for PCP to maintain "both seeing the forest and clearly seeing every tree" during extremely long interactions.

### 7.1 Semantic View States

To allow the model to intuitively perceive reading depth and physical attributes (atomic level vs. container level), PCP employs a semantic view system:

1.  **`view="Summary"` (Summary)**: Shows logical extraction, hiding underlying data.
2.  **`view="Detail"` (Detail)**: Shows the full content of the page.
    *   **Original Node (Atomic)**: The logical endpoint, cannot be further deconstructed.
    *   **Consolidated Node (Container)**: A logical pivot point, can be further deconstructed.
3.  **`view="Unpacked"` (Unpacked)**: 
    *   **Constraint**: **Only applicable to `type="Consolidated"` nodes**. Must satisfy the "Active Zoom" constraint: it must contain **at least one** sub-node in a state other than `Summary` (i.e., `Detail` or deeper), otherwise it should automatically trigger a `Shelve` back to the `Detail` state to keep the context compact.
    *   **Behavior**: Removes the synthesis text and directly nests internal sub-pages (`Node`).

### 7.2 Recursive Zooming Path Mapping

*   **Level 1: Global Perception**: Root pages in the horizon are presented in the `Summary` state to build a logical overview.
*   **Level 2: Node Penetration**: `Consult(id)` moves the target into `Detail`.
    *   **Original**: Reveals materialized full content.
    *   **Consolidated**: Reveals the full text of the container summary.
*   **Level 3: Sub-tree Unpacking**: Calls `Consult` on a **Consolidated** node that is already in `Detail` state, moving it to `Unpacked` to reveal the `Summaries` of its internal child pages.
    *   **Recursive Trait**: Since child pages can be new Consolidated nodes, this process supports **infinite recursive zooming**, allowing for vertical searches within multi-level logic trees.
*   **Level 4: Atomic Restoration**: Calling `Consult` to move an `Original` page at the edge of the tree into `Detail`, reaching the logical endpoint.

### 7.3 Protocol Instruction Specs

| Instruction | Call Format | Trigger Condition | Effect |
| :--- | :--- | :--- | :--- |
| **Consult** | `Consult(reason, id)` | Resolution of an existing logical entity is insufficient | **Logical Upgrade**: `Summary -> Detail` or `Detail -> Unpacked`. Target is a known LAS ID. |
| **Explore** | `Explore(reason, handle, keywords)` | Need to extract specific logic from an unknown physical block | **Physical Materialization**: Filters and generates new Pages from a PBlock handle based on keywords. Target is a PAS handle. |
| **Shelve** | `Shelve(reason, id)` | Current detail node is no longer relevant | **View Downgrade**: `Unpacked -> Detail` or `Detail -> Summary`. |

### 7.4 Cascading Shelve / Auto-Folding

To maintain extreme context purity, `Shelve` operations possess **cascading folding** characteristics:
*   **Atomic-level Trigger**: When an atomic Page in a `Detail` state is `Shelved`, it immediately reverts to `Summary`.
*   **Container-level Collapse**: When a summary page in Level 3 (Unpacked state) has all its internal sub-page IDs successfully `Shelved` (folded) by the Worker, the summary page node must **automatically collapse upward**, reverting to the Level 2 (Detail) state.
*   **Logical Goal**: Ensure that only "explicitly needed details" exist in the cognitive horizon, leaving no logical redundancy.

## VIII. Physical Mapping & Search Logic

This section defines how PBlocks are transformed into Pages and the physical search details behind the `Explore` instruction.

### 8.1 Keyword-Driven Entropy Suppression
*   **Semantic Filter**: `Explore` does not blindly load full physical content. The Host performs semantic retrieval (BM25 or Vector) within the PBlock based on `keywords`, extracting only segments strongly relevant to the keys.
*   **Entropy Suppression**: Physical noise falling below relevance thresholds is forcibly kept in the "physical space" and not materialized. This ensures an extremely high signal-to-noise ratio in the logical address space.

### 8.2 Structural Awareness
The Host determines the materialization form based on the physical attributes of the PBlock:
*   **Atomic Physical Block** (e.g., text logs, function snippets) -> Materialized as an **Original Page**.
*   **Structured Physical Block** (e.g., repository directories, complex document chapters) -> Materialized as a **Consolidated Page (Draft)**.
    *   **Physical Zoomming**: Calling `Consult` on a structured Draft triggers a **sub-physical block scan**, producing the next level of draft pages, unifying physical and logical paths during zooming.

### 8.3 Recursive Synthesis
When materializing a complex structured PBlock, the Consolidator performs **recursive synthesis**:
1.  Calculates key summaries of sub-physical blocks bottom-up.
2.  Summaries at each level are merged step-by-step, eventually producing the `summary` and `keywords` for the Root Consolidated Page.
3.  This process is highly **intent-dependent**—the emphasis of the Summary is determined by the current `Intent Focus`.

**Parameter Descriptions**:
*   **reason**: Logical reason for the operation.
*   **id / handle**: Target Logic ID or Physical Handle.
*   **keywords**: Exclusive to `Explore`, defines the semantic filter for physical probing.

*   **Logical Goal**: Ensure that only "explicitly needed details" exist in the cognitive horizon, leaving no logical redundancy.

## IX. XML Spec

### 9.1 XML Synthesis vs. Semantic Gluing

PCP does not simply stack text physically (Plain Text Gluing) but builds a perceptional interface with structural determinism through **Ordered XML Synthesis**. The core design motivations include:

*   **Structural Disambiguation**: When dealing with context containing complex metadata (e.g., ID, Timestamp, Page Type), XML provide explicit logical boundaries. This effectively prevents the LLM from cognitively confusing "protocol control instructions" with "actual dialogue content," reducing hallucination drift during reasoning.
*   **Addressing Bus & Operational Closure**: XML tags provide deterministic addressing targets (IDs) for `Consult` and `Shelve` instructions. This allows the external system to perform precise local insertions or deletions on the context—much like manipulating the DOM—without needing to refresh the entire context flow.
*   **Parsing Determinism**: Modern long-context models exhibit excellent adherence to structural markers. Treating the context as an "addressable database" rather than a "flat text stream" helps the model maintain rigorous logical tracing capabilities.

### 9.2 Tag Specifications

*   **`<PagedContext>`**: Protocol root container, carries `version` number.
*   **`<Static_Registry>`**: Static registry. Injects global constants and **runtime instructions** that do not change with the dialogue.
    *   **`<ST-Node>`**: **Status Node**. Stores system states or global parameters that do not change during the dialogue. Includes `id` and `value`.
    *   **`<System_Instructions>`**: **Core instruction injection**. Informs the Worker of the protocol's operating manual, defining triggers and logical goals for `Consult/Shelve`.
*   **`<Query>`**: **Current user input**. The system performs intent recognition and cascading matching based on this input.
*   **`<Reasoning_Trace>`**: **Reasoning process record**. Consists of a series of `<Step>` nodes recording all zooming actions taken by the Worker in previous rounds.
    *   **`<Step>`**: Specific action item. Includes `action` (action name), `target` (target ID), and `reason` (logical motivation).
*   **`<Linear_Flow>`**: **Linear memory flow**. A collection of Page nodes representing the current "viewable content."
*   **`<Node>`**: Logical page container.
    *   `id`: unique Short Hash identifier.
    *   `type`: **Physical page attribute (Original | Consolidated)**.
    *   `view`: **Semantic view (Summary | Detail | Unpacked)**.
    *   `depth`: **Hierarchy Scale (Integer)**.
    *   `origin`: **Origin Tracking (History | Storage)**.
    *   `keywords`: **Semantic Index Keys (Comma-separated string)**.
    *   `timestamp`: The time origin of the logical occurrence.

---

### 9.3 Abstract Boilerplate

Standard generation template for system integration:

```xml
<PagedContext version="0.1.0-alpha">
  <Static_Registry>
    <ST-Node id="CURRENT_TIME" value="YYYY-MM-DDTHH:mm:ss" />
    <System_Instructions>
      - Original: original evidence/dialogue node (non-deconstructible).
      - Consolidated: logical synthesis node (can be Unpacked).
      - view="Summary": abstract; view="Detail": full text; view="Unpacked": expand container internals.
      - Consult(reason, id): upgrade view for details/sub-items; Shelve(reason, id): downgrade view to clean horizon.
    </System_Instructions>
  </Static_Registry>

  <Query>...</Query>

  <Reasoning_Trace>
    <Step action="Consult" target="PageID" reason="Seeking original evidence support behind high-level logic." />
  </Reasoning_Trace>

  <Linear_Flow>
    <!-- 1. Original Summary -->
    <Node id="f1a2b3c4" type="Original" view="Summary" depth="1" keywords="CS, Addressing" origin="History" timestamp="2026-02-14T10:00:00">
      <Summary>Background reference summary...</Summary>
    </Node>

    <!-- 2. Original Detail - Logical Endpoint -->
    <Node id="d4e5f6a1" type="Original" view="Detail" depth="2" origin="Storage" timestamp="2026-02-14T10:05:00">
      <Content>Full bottom-level dialogue text or hardware raw logs...</Content>
    </Node>

    <!-- 3. Consolidated Summary -->
    <Node id="b2c3d4e5" type="Consolidated" view="Summary" depth="1" timestamp="2026-02-14T10:10:00">
      <Summary>Preliminary consensus summary synthesized from 10 OP pages.</Summary>
    </Node>

    <!-- 4. Consolidated Detail - Recursive Transit -->
    <Node id="c3d4e5f6" type="Consolidated" view="Detail" depth="1" timestamp="2026-02-14T10:15:00">
      <Content>Consolidated full text, containing complete logical deductions. Drilling down is possible here.</Content>
    </Node>

    <!-- 5. Consolidated Unpacked - Direct Nesting -->
    <Node id="e5f6a7b8" type="Consolidated" view="Unpacked" depth="1" timestamp="2026-02-14T10:20:00">
       <Node id="a7b8c9d0" type="Original" view="Summary" depth="2" origin="Storage" timestamp="..." />
       <Node id="f9e8d7c6" type="Original" view="Summary" depth="2" origin="Storage" timestamp="..." />
    </Node>
  </Linear_Flow>
</PagedContext>
```

---

## X. Sentry Mechanism & Logic Collapse

### 10.1 Sentry Logic
The system monitors token pressure $P_{token}$ and input entropy in real-time. When over-limit pressure or massive new file input is detected, the Sentry forces the system from "Chat Mode" to "Protocol Mode."

### 10.2 Retroactive Transition (Logic Collapse)
At the moment of transition, the system performs a **dehydration process** on the previous linear dialogue history:
*   **Retroactive Paging**: Retrospectively applies PCP paging logic to the previous linear history. Depending on topic pivots, token budgets, and logic density, it collapses the history into a series of protocol-compliant `Original/Consolidated Pages`, ensuring logical continuity.
*   **Physical Cleansing**: Clears physical cache and establishes a formal XML addressing bus.
*   **Identity Collapse**: The Worker's identity formally collapses from "chat partner" into a **"Logic Processor."**

---

## XI. Logic Persistence & Write-back

PCP is not responsible for maintaining a networked cold knowledge base, but as a "Logic Processor," it must provide a mechanism to sediment the "Logical Net Value" generated during real-time processing back into external systems.

### 11.1 Memory Interface Requirements
PCP is indifferent to the internal implementation of the Memory system, but as an external "logic plug-in," it must satisfy the following calling contracts:
*   **Page Message Compatibility**: The content returned by Memory must comply with the `Original/Consolidated Page` XML definition, containing core `summary` and `keywords` for further alignment by the Router.
*   **Keyword Interface**: Memory must expose an interface that supports Recall based on semantic keywords. The Router extracts intent keywords during the Addressing phase and calls this interface in parallel.
*   **Instant Input Role**: Memory acquisition is part of the addressing flow. Retrieved Pages are treated as valid extensions of the current LAS, enjoying the same logical processing weight as local cache.

### 11.2 Unified Logic Export
When the task stream reaches a stable logical conclusion or significant "logical extraction" occurs (such as generating high-depth Consolidated Pages), the **Unified Export Manager** of the Host system is triggered:
*   **Export Assets**: PCP throws out the `summary`, `keywords`, and established `source_ids` logical associations processed by the processor.
*   **External Sedimentation Policy**: Whether the content is stored in Memory for real-time addressing in subsequent sessions or sedimented to generic RAG as non-linear background facts is **entirely decided by the receiver (Memory/RAG system)**. PCP acts only as a producer of logical net value and does not interfere with the policy execution of external storage systems.

### 11.3 State Isolation
*   **Non-blocking Write-back**: The persistence process is transparent and asynchronous to the real-time PCP task stream.
*   **Side-effect Free**: Updates to the external networked knowledge base do not cause real-time changes to the current PCP Linear_Flow unless the Worker explicitly addresses these new sedimented physical blocks via the Router in the next round.

---
