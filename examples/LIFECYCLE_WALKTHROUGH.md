# PCP 全生命周期实战演示：能源转型战略分析
# Full Lifecycle Case Study: Energy Transition Strategy

本案例严格按照 PCP 协议第五章定义的**“五阶段运行模型”**，演示系统如何从冷启动感知到最终记忆固化的全过程。

---

## 阶段 1：加载感知 (Perception)
**场景**：用户输入 Query，系统从长期记忆（Index）中检索候选节点。
- **输入**: “为重工业选择绿氢电解槽，要求寿命 >5万小时且成本低。”
- **检索动作**: 扫描逻辑库，发现两页综述（SOEC, AEM）及一页项目背景。

---

## 阶段 2：级联分压 (Cascade)
**逻辑**：Router 执行意图匹配与广义初筛。
- **意图重构**: 基于用户提问，识别出“寿命”与“成本”为双核心权重指标。
- **筛选决策**: 判定 `SOEC` (CP1) 与 `AEM` (CP2) 为高相关背景。

---

## 阶段 3：合成与注入 (Synthesis)
**状态**：系统将物理排序后的节点合成为 XML 注入 Worker 缓冲区。

```xml
<PagedContext version="1.0">
  <Static_Registry>
    <System_Instructions>...</System_Instructions>
  </Static_Registry>
  <Query>为重工业选择绿氢电解槽，要求寿命 >5万小时且成本低。</Query>
  <Linear_Flow>
    <Node id="s1o2e3c4" type="Consolidated" view="Summary">
      <Summary>SOEC 技术综述：侧重高温能效，但存在热应力衰减挑战。</Summary>
    </Node>
    <Node id="a1e2m3n4" type="Consolidated" view="Summary">
      <Summary>AEM 技术综述：侧重低 CAPEX 优势与非贵金属催化剂寿命。</Summary>
    </Node>
    <Node id="p1j2k3l4" type="Original" view="Detail">
      <Content>项目现状：要求 2027 年投产，需满足极高可靠性约束。</Content>
    </Node>
  </Linear_Flow>
</PagedContext>
```

---

## 阶段 4：交互执行 (Execution - 包含分支变焦)
**动作 1：下钻路径 A (SOEC)**
Worker 怀疑高效率的 SOEC 可能是解。执行 `Consult` 下钻详情。
**XML 变化**: `s1o2e3c4` 进入 `Unpacked` 态。

```xml
<PagedContext version="1.0">
  <Query>为重工业选择绿氢电解槽，要求寿命 >5万小时且成本低。</Query>
  <Reasoning_Trace>
    <Step action="Consult" target="s1o2e3c4" reason="优先查验 SOEC 模块的工业运行寿命上限" />
  </Reasoning_Trace>
  <Linear_Flow>
    <Node id="s1o2e3c4" type="Consolidated" view="Unpacked">
      <Node id="r1c2o3r4" type="Original" view="Detail" timestamp="...">
        <Content>实验数据：目前 SOEC 连续运行寿命约为 1.8万小时，无法满足 5万小时要求。</Content>
      </Node>
    </Node>
    <Node id="a1e2m3n4" type="Consolidated" view="Summary" />
    <Node id="p1j2k3l4" type="Original" view="Detail" />
  </Linear_Flow>
</PagedContext>
```

**动作 2：路径回退 (Shelve & Backtrack)**
发现 SOEC 不达标，执行 `Shelve` 回退分支。触发**自动坍缩**。

```xml
<PagedContext version="1.0">
  <Query>为重工业选择绿氢电解槽，要求寿命 >5万小时且成本低。</Query>
  <Reasoning_Trace>
    <Step action="Consult" target="s1o2e3c4" reason="优先查验 SOEC 寿命上限" />
    <Step action="Shelve" target="r1c2o3r4" reason="SOEC 寿命（1.8万小时）不满足项目约束，分支排除" />
  </Reasoning_Trace>
  <Linear_Flow>
    <Node id="s1o2e3c4" type="Consolidated" view="Summary" />
    <Node id="a1e2m3n4" type="Consolidated" view="Summary" />
    <Node id="p1j2k3l4" type="Original" view="Detail" />
  </Linear_Flow>
</PagedContext>
```

**动作 3：下钻路径 B (AEM - 最终锁定)**
转向 AEM 分支。锁定其 >6万小时的可靠性证据。

```xml
<PagedContext version="1.0">
  <Query>目前建议使用哪种技术？</Query>
  <Reasoning_Trace>
    <Step action="Shelve" target="r1c2o3r4" reason="排除 SOEC 分支" />
    <Step action="Consult" target="a1e2m3n4" reason="转向 AEM 技术下钻其长效催化剂稳定性证据" />
  </Reasoning_Trace>
  <Linear_Flow>
    <Node id="s1o2e3c4" type="Consolidated" view="Summary" />
    <Node id="a1e2m3n4" type="Consolidated" view="Unpacked">
      <Node id="a1e2m3_det" type="Original" view="Detail" timestamp="...">
        <Content>测算：新型镍基催化剂在工业工况下，外插运行寿命可达 6.5万小时。</Content>
      </Node>
    </Node>
    <Node id="p1j2k3l4" type="Original" view="Detail" />
  </Linear_Flow>
</PagedContext>
```

---

## 阶段 5：记忆固化 (Memory Consolidation)
**逻辑**：任务完成后，Consolidator 执行现场清理。
- **固化 (Freezing)**：将本次推演形成的决策逻辑（包含 SOEC 被排除的原因及 AEM 的选用理由）抽象为新的 `Consolidated Page`，替换掉内存中的 `p1j2k3l4`。
- **剪裁 (Pruning)**：将不再活跃的 `s1o2e3c4` 推入背景远场记忆，释放当前视界。
