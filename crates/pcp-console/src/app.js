import { createPageInspector } from "/page-inspector.js?v=20260816.3";
import { describePagePayload, pagePayloadPreviewText, renderPagePreview } from "/page-content.js?v=20260816.3";
import { createHealthView } from "/health-view.js?v=20260816.3";
import { createRetentionView } from "/retention-view.js?v=20260818.1";
import { createQueryView } from "/query-view.js?v=20260818.8";

const DEFAULT_PAGE_LIMIT = 20;
const PAGE_LIMIT_OPTIONS = new Set([10, 20, 30]);
const ACCESS_LIMIT = 50;
// Local summary workers receive one Page at a time so an incomplete response
// affects only that Page and can be retried independently.
const SUMMARY_REVIEW_BATCH_SIZE = 1;
const THEME_STORAGE_KEY = "pcp-console.theme";
const LANGUAGE_STORAGE_KEY = "pcp-console.language";
const PAGE_LIMIT_STORAGE_KEY = "pcp-console.pages-per-page";
const ZH_MESSAGES = {
  "24 hours": "24 小时",
  "7 days": "7 天",
  "30 days": "30 天",
  "90 days": "90 天",
  "Access": "访问",
  "Access audit": "访问审计",
  "active": "活跃",
  "Activity trend": "活动趋势",
  "A read-only dry run over historical Revisions. Current Page heads and sealed evidence cannot become candidates.": "对历史修订进行只读试算。当前页面头版本和已封存证据不会成为候选项。",
  "Structural signals are descriptive only. Scan evaluates the full eligible inventory before each maintenance phase.": "结构信号只用于描述。每个维护阶段开始前，扫描会检查全部符合条件的页面库存。",
  "All scopes": "所有范围",
  "Analyze": "分析",
  "Analyze suggestions": "分析建议",
  "Pack maintenance": "打包维护",
  "Semantic maintenance": "语义维护",
  "Continue to semantic maintenance": "继续语义维护",
  "Analysis failed": "分析失败",
  "Analysis incomplete": "分析未完成",
  "Automatic maintenance": "自动维护",
  "Settings": "设置",
  "General": "通用",
  "Settings sections": "设置分区",
  "Automatic maintenance settings": "自动维护设置",
  "Changes restart the Runtime. Suggested relations still require review.": "保存后会重启 Runtime；建议关联仍需人工审核。",
  "Enable automatic maintenance": "启用自动维护",
  "When disabled, Runtime records no automatic maintenance heartbeat.": "禁用时，Runtime 不会记录自动维护心跳。",
  "Mode": "模式",
  "Observe": "观察",
  "Apply": "应用",
  "New Pages": "新增页面数",
  "Quiet period (minutes)": "静默期（分钟）",
  "Maximum wait (minutes)": "最长等待（分钟）",
  "Maximum wait must be at least the quiet period, and all values must be positive.": "最长等待必须不小于静默期，且所有值必须为正数。",
  "Save and restart Runtime": "保存并重启 Runtime",
  "Runtime-owned write-trigger state. It never makes suggested relations retrievable.": "由 Runtime 持有的写入触发状态；建议关联不会因此参与检索。",
  "Not started": "尚未开始",
  "Waiting": "等待写入",
  "Running": "正在执行",
  "Failed": "失败",
  "Stale": "状态过期",
  "Disabled": "已禁用",
  "Observed heads": "已观测页面头",
  "Dirty regions": "待整理范围",
  "Ready regions": "已就绪范围",
  "Pending relation review": "待审关联",
  "Uncertain relation": "不确定关联",
  "Manual approval required": "需要人工批准",
  "Expand full Page": "展开完整页面",
  "Loading full Page…": "正在加载完整页面…",
  "Open in inspector": "在检查器中打开",
  "Write trigger": "写入触发条件",
  "Last completed": "最近完成",
  "Awaiting the first Runtime heartbeat.": "等待 Runtime 首次心跳。",
  "Approve": "批准",
  "Analyzing": "正在分析",
  "Batches completed": "已完成批次",
  "Pages completed": "已完成页面",
  "Failed batches": "失败批次",
  "Failed pages": "失败页面",
  "Current Pages": "当前页面",
  "Appearance and language": "外观与语言",
  "Applications requesting access to this PCP Store": "正在请求访问此 PCP Store 的应用",
  "Approved clients": "已批准客户端",
  "Auto": "自动",
  "Back": "返回",
  "Candidates": "候选项",
  "Proposals": "提案",
  "Cancel": "取消",
  "Capabilities": "能力",
  "Calls": "调用",
  "Change": "变更",
  "Chinese": "中文",
  "Client access": "客户端访问",
  "Close": "关闭",
  "Connected": "已连接",
  "Confirm action": "确认操作",
  "Connecting": "正在连接",
  "Connections": "关联",
  "Console controls": "控制台操作",
  "Console views": "控制台视图",
  "content": "内容",
  "Content": "内容",
  "Continue": "继续",
  "Created": "创建时间",
  "Current": "当前",
  "Dark": "深色",
  "days": "天",
  "deferred": "已延后",
  "Decision": "决策",
  "Decisions": "决策",
  "Degraded": "降级",
  "Eligible historical Revisions": "可处理的历史修订",
  "Eligibility requires both age and absence of every protection root. Preview rows only limits the tables below.": "候选项必须同时满足年龄条件且不存在任何保护根。预览行数只限制下方表格。",
  "English": "English",
  "Endpoint": "端点",
  "Estimated size": "估算大小",
  "Estimated calls": "预计调用",
  "estimated model calls": "预计模型调用",
  "Exact": "精确",
  "Expires": "到期时间",
  "Explicit retention leases": "显式保留租约",
  "Failures": "失败",
  "Follow system": "跟随系统",
  "Granted scopes": "授权范围",
  "Health": "运行状态",
  "Health analysis window": "健康分析范围",
  "History": "历史",
  "Historical revision cleanup": "历史版本回收",
  "historical Revisions may be reclaimable under current safeguards.": "个历史修订在当前保护条件下可能可回收。",
  "No historical Revision is reclaimable under current safeguards.": "当前保护条件下没有可回收的历史修订。",
  "Holder": "持有者",
  "Identity": "身份",
  "How clients retrieve, revise, and contract the current memory surface.": "客户端如何检索、修订和收缩当前记忆表面。",
  "Inputs": "输入",
  "Integrity": "完整性",
  "Issue": "问题",
  "issues": "个问题",
  "Keep newest per Page": "每页保留最新",
  "Kind": "类型",
  "Language": "语言",
  "Light": "浅色",
  "Load more": "加载更多",
  "Maintenance": "维护",
  "Meaning": "含义",
  "model calls": "次模型调用",
  "Minimum candidate age in days": "候选项最小年龄（天）",
  "Not loaded": "未加载",
  "Not scanned": "未扫描",
  "No events": "没有事件",
  "No eligible work": "没有可处理内容",
  "No pages": "没有页面",
  "No proposals": "没有提案",
  "Relation review queue": "关联审核队列",
  "These suggested links are not used for retrieval until you approve them.": "这些建议关联在你批准前不会参与检索。",
  "Review evidence": "审核依据",
  "Reject": "拒绝",
  "Suppress": "抑制",
  "No preview": "没有预览",
  "Preview unavailable": "预览暂不可用",
  "New Pack": "新建打包页",
  "no grants": "无授权范围",
  "Observed": "观测时间",
  "Overview": "概览",
  "Older than": "早于",
  "Operation": "操作",
  "Open": "打开",
  "Operational health": "运行状态",
  "Operations": "操作",
  "Optimize": "优化",
  "Optimization completed": "优化完成",
  "Origin": "来源",
  "Order pages": "页面排序",
  "Output": "输出",
  "Pack candidates": "Pack 候选项",
  "Pack": "打包",
  "Pack work": "打包工作",
  "Page": "页面",
  "Scope and source": "范围与源",
  "Page views": "页面视图",
  "Pages": "页面",
  "Query": "查询",
  "No query yet": "尚未查询",
  "Search all authorized context, then review the deterministic context pack.": "检索所有已授权上下文，再审阅确定性组装的 Context Pack。",
  "Search or intent": "搜索内容或完整意图",
  "Describe what you need to recall": "描述你希望找回的内容",
  "Retrieval method": "检索方法",
  "Semantic search": "语义搜索",
  "Match intent": "意图匹配",
  "All authorized scopes": "所有已授权范围",
  "Top results": "结果数量",
  "Build context pack": "组装 Context Pack",
  "Context pack review": "Context Pack 审阅",
  "Run a query to inspect the ranked context pack.": "执行查询以审阅按相关度排序的 Context Pack。",
  "Search ranks literal matches and assembles the selected results without inference.": "关键词搜索按字面匹配排序，并在不进行推断的情况下组装结果。",
  "Full payload": "完整内容",
  "Excerpt": "内容片段",
  "Current summary": "当前摘要",
  "Reference": "引用",
  "Anchor": "锚点",
  "anchors": "个锚点",
  "related": "个关联上下文",
  "Related context": "关联上下文",
  "Asserted relation": "明确关联",
  "from anchor": "来自锚点",
  "Incoming relation": "入边",
  "Outgoing relation": "出边",
  "Inclusion reason": "纳入理由",
  "Literal match in": "字面命中",
  "Ranked": "排名",
  "Focus layer": "重点层",
  "Summary layer": "摘要层",
  "Reference layer": "引用层",
  "Current summary is tied to the selected revision.": "当前摘要已校验属于选中的 revision。",
  "Included as a reference after detailed context.": "详细上下文之后，作为引用保留。",
  "Open source page": "打开来源页面",
  "Reference and provenance": "引用与溯源",
  "Revision": "Revision",
  "Provenance": "溯源",
  "Context sent to model": "发送给模型的 Context",
  "context entries": "条上下文",
  "This is the exact deterministic context assembled by PCP for the model.": "这是 PCP 将实际交给模型的确定性 Context。",
  "Source projection was incomplete; PCP downgraded it before packing.": "来源 projection 未完整读取；PCP 已在组装前降级处理。",
  "results": "条结果",
  "char budget": "字符预算",
  "pages": "页",
  "Pending approval": "待批准",
  "Preparing": "正在准备",
  "Principal": "主体",
  "Principals": "主体",
  "Principal type": "主体类型",
  "Protocol": "协议",
  "proposals": "个提案",
  "Preview rows": "预览行数",
  "Protected historical Revisions": "受保护的历史修订",
  "Raw": "原始数据",
  "ready": "已就绪",
  "Re-analyze this phase": "重新分析此阶段",
  "Relation work": "关联工作",
  "Relations": "关联",
  "Explicit relation": "显式关联",
  "Reason": "原因",
  "Reasons": "原因",
  "Reasons overlap; one Revision can match several rows.": "保护原因可以重叠；一个修订可能匹配多行。",
  "Recent data quality sample": "近期数据质量样本",
  "Recent events": "近期事件",
  "Recent activity": "最近活动",
  "Oldest first": "最早记录",
  "Most direct links": "直接关联最多",
  "Fewest direct links": "直接关联最少",
  "Largest content": "内容最大",
  "Source order": "源顺序",
  "Search results are ranked by relevance": "搜索结果按相关性排序",
  "Refresh": "刷新",
  "Restart Runtime": "重启运行时",
  "Retention": "保留",
  "Retention collection": "保留清理",
  "Retention leases": "保留租约",
  "Retention planning": "保留规划",
  "Revision retention plan": "修订保留计划",
  "Revisions": "修订",
  "Run dry run": "运行试算",
  "Runtime activity and stored-memory shape from metadata. These signals do not judge whether content is true or relevant.": "基于元数据展示运行时活动和已存记忆形态。这些信号不判断内容是否真实或相关。",
  "Runtime behavior": "运行时行为",
  "Runtime PID": "运行时 PID",
  "Runtime started": "运行时启动时间",
  "Restart the PCP Runtime managed by this Console": "重启由此 Console 管理的 PCP Runtime",
  "Sample findings": "样本发现",
  "Scan": "扫描",
  "Scan failed": "扫描失败",
  "Scanned": "已扫描",
  "Scanning": "正在扫描",
  "Scanning current Pages": "正在扫描当前页面",
  "Start maintenance": "开始维护",
  "Start a maintenance session": "发起维护会话",
  "A maintenance session scans the full eligible Store one stage at a time. Analysis and writes require your explicit action.": "维护会话会逐阶段扫描完整的可处理库存。分析和写入都需要你明确发起。",
  "Each maintenance type first scans structural candidates, then analyzes them with a model, then waits for your explicit application. Suggested links are never retrievable before approval.": "每类维护都会先扫描结构候选，再由模型分析，最后等待你明确应用；建议关联在批准前绝不参与检索。",
  "Maintenance policy": "维护策略",
  "Scheduled": "已调度",
  "Manual": "手动",
  "Stage": "阶段",
  "Waiting": "等待中",
  "Not started": "未开始",
  "Completed": "已完成",
  "Ready to analyze": "可开始分析",
  "Ready to apply": "可应用提案",
  "Ready to continue": "可继续",
  "Scan complete": "扫描完成",
  "Scan candidates": "扫描候选",
  "Review and apply": "审阅应用",
  "Analyze Pack": "分析打包",
  "Analyze Summary": "分析摘要",
  "Analyze Relations": "分析关联",
  "Apply selected": "应用所选",
  "Continue to Summary": "继续到摘要",
  "Continue to Relations": "继续到关联",
  "Complete maintenance": "完成维护",
  "Rescan this stage": "重新扫描本阶段",
  "End session": "结束会话",
  "Maintenance report": "维护报告",
  "This session": "本次会话",
  "Applied": "已应用",
  "Skipped": "已跳过",
  "Skip this stage": "跳过本阶段",
  "No changes were applied": "没有应用变更",
  "Model calls": "模型调用",
  "Eligible work in this stage": "本阶段可处理库存",
  "Scanned candidate groups": "扫描到的候选组",
  "Scanned eligible pages": "扫描到的待处理页面",
  "Review diagnostics": "查看诊断",
  "Retry failed batches": "重试失败批次",
  "Retry failed pages": "重试失败页面",
  "Open page": "打开页面",
  "Summary proposal": "摘要提案",
  "Diagnostics are separate from the current maintenance step and do not start model work.": "诊断与当前维护步骤分离，不会发起模型工作。",
  "Current stage": "当前阶段",
  "Analysis completed. No changes are recommended for this stage. Continue when you are ready.": "分析完成：本阶段没有需要优化的内容。准备好后可继续。",
  "Review the proposals below, select the changes to apply, then continue to the next stage.": "复核下方提案，选择要应用的变更，再继续到下一阶段。",
  "End this maintenance session?": "结束本次维护会话？",
  "No additional Page writes will occur. Unapplied proposals remain unapplied.": "不会再写入页面；未应用的提案将保持未应用。",
  "Maintenance session started": "维护会话已开始",
  "Maintenance session completed": "维护会话已完成",
  "Maintenance runs in two passes: first Pack boundaries, then summaries and relations on the refreshed inventory. Each pass scans candidates, analyzes suggestions, then waits for your explicit application. Suggested links are never retrievable before approval.": "维护分为两段：先处理 Pack 边界，再基于刷新后的库存处理摘要与关联。每段都会扫描候选、分析建议，并等待你明确应用；建议关联在批准前绝不参与检索。",
  "Scope": "范围",
  "Scopes": "范围",
  "Search": "搜索",
  "Search method": "搜索方式",
  "Search pages": "搜索页面",
  "Searches": "搜索",
  "Session": "会话",
  "Select all candidates": "全选候选项",
  "Share of scanned": "扫描占比",
  "Size": "大小",
  "Source": "来源",
  "Source stream": "源流",
  "Stream": "流",
  "Summarized": "已摘要",
  "Technical pages": "技术页面",
  "This Console does not own the current Runtime": "此 Console 不拥有当前 Runtime",
  "Text": "文本",
  "Theme": "主题",
  "Time": "时间",
  "Updated": "更新时间",
  "Unavailable": "不可用",
  "Available": "可用",
  "Audit timeline": "审计时间线",
  "Conversation pack": "对话 Pack",
  "explicit": "显式",
  "Image": "图像",
  "in": "入",
  "Isolated": "孤立",
  "loaded": "已加载",
  "shown": "已显示",
  "Loading": "正在加载",
  "Loading more": "正在加载更多",
  "Load failed": "加载失败",
  "Activity time": "活动时间",
  "Ascending": "正序",
  "Descending": "逆序",
  "Filter Pages": "筛选页面",
  "Next page": "下一页",
  "Page number": "第",
  "Page results": "页面结果",
  "Pages pagination": "页面分页",
  "Previous page": "上一页",
  "Results per page": "每页展示条目",
  "Changing this reloads the current page list.": "更改后会重新加载当前页面列表。",
  "Sort by": "排序条件",
  "Sort direction": "排序方向",
  "Sort Pages": "排序页面",
  "Lossless page packing": "无损页面 Pack",
  "out": "出",
  "Signal": "信号",
  "Structure": "结构",
  "Direct links": "直接关联",
  "No direct links": "没有直接关联",
  "No recorded linkage": "未记录关联",
  "Source positions": "源位置",
  "Packed": "已打包",
  "Window": "时间范围",
  "All connections": "所有关联",
  "Created by": "创建者",
  "Explicit relations": "显式关联",
  "hop": "跳",
  "hops": "跳",
  "Mutability": "可变性",
  "No content projection": "没有内容投影",
  "No validity assessment": "没有有效性评估",
  "Node budget": "节点上限",
  "Status": "状态",
  "Summary": "摘要",
  "Summary route": "摘要路径",
  "Summary content": "摘要内容",
  "Summary page": "摘要页面",
  "Summary work": "摘要工作",
  "Summarizes": "摘要目标",
  "Traversal depth": "遍历深度",
  "Validity": "有效性",
  "Why Revisions are protected": "为什么修订受保护",
  "Writes": "写入",
  "Active temporary holds": "活跃临时保留",
  "Authorized client operations in this window": "所选时间范围内已授权的客户端操作",
  "A recent active Page sample for maintenance. These are routing and compression opportunities, not semantic verdicts.": "近期活跃页面的维护样本。这些是路由与压缩机会，不构成语义判断。",
  "A read-only plan over historical Revisions. No revision is deleted here; current Page heads, sealed evidence, and required provenance remain protected.": "对历史修订进行只读规划。此处不会删除修订；当前页面头版本、已封存证据和必要溯源仍受保护。",
  "calls": "次调用",
  "Collecting": "采集中",
  "Dense explicit-relation neighborhood": "显式关联过于密集",
  "denied calls need inspection.": "次拒绝调用需要检查。",
  "Estimated storage": "估算存储空间",
  "Explicit-relation coverage": "显式关联覆盖率",
  "Failed / denied": "失败 / 拒绝",
  "failed and": "次失败，",
  "failures": "次失败",
  "Finite holds on revision history": "历史修订上的有限期保留",
  "Historical revisions still required by a protection root": "仍被保护根要求保留的历史修订",
  "historical Revisions have no protection root under this preview policy.": "个历史修订在当前预览策略下没有保护根。",
  "historical Revisions remain protected.": "个历史修订仍受保护。",
  "Long Page has no explicit relation": "长页面没有显式关联",
  "Long Page has no Summary route": "长页面没有摘要路径",
  "Long-page Summary route": "长页面摘要路径",
  "Long Pages": "长页面",
  "Review summary": "复核摘要",
  "Review relation": "复核关联",
  "Action": "操作",
  "measured": "已测量",
  "measured calls": "次已测量调用",
  "Memory structure review": "记忆结构复核",
  "Memory maintenance": "记忆维护",
  "Measured response latency": "已测量的响应延迟",
  "most recent active Pages sampled": "个最近活跃页面已采样",
  "No active temporary holds": "没有活跃临时保留",
  "No historical Revision is reclaimable under this preview policy.": "当前预览策略下没有可回收的历史修订。",
  "No maintenance signals in the recent active sample": "近期活跃样本中没有维护信号",
  "No reclaimable historical Revisions under this policy": "此策略下没有可回收的历史修订",
  "No workload activity in this window": "此时间范围内没有工作负载活动",
  "No workload calls were observed in this window. Telemetry begins with the upgraded runtime.": "此时间范围内未观测到工作负载调用。遥测从升级后的运行时开始记录。",
  "No workload operations in this window": "此时间范围内没有工作负载操作",
  "Observed calls": "观测到的调用",
  "Observed client activity, response performance, and telemetry coverage. This view does not evaluate recall relevance.": "观测客户端活动、响应性能和遥测覆盖率。本视图不评估召回相关性。",
  "Observed retrieval and revision traffic. Counts describe use, not recall quality.": "观测到的检索与修订流量。计数描述使用情况，不描述召回质量。",
  "Observed telemetry coverage and response latency.": "观测到的遥测覆盖率与响应延迟。",
  "of calls in this window include detailed telemetry.": "的调用包含详细遥测。",
  "of measured searches returned at least one Page. This does not establish relevance.": "的已测量搜索至少返回了一个页面。这不代表其相关。",
  "of measured searches returned no Page.": "的已测量搜索没有返回页面。",
  "p50 latency": "p50 延迟",
  "p95 latency": "p95 延迟",
  "Pages returned": "返回页面",
  "Plan failed": "规划失败",
  "Planning": "正在规划",
  "Open to check": "展开以检查",
  "Preview policy": "预览策略",
  "preview updated": "预览更新于",
  "Protected historical samples": "受保护的历史样本",
  "Protected history": "受保护历史",
  "Protection details": "保护详情",
  "Reclaimable historical revision content": "可回收的历史修订内容",
  "Reclaimable history": "可回收历史",
  "Reclaimable revision history": "可回收修订历史",
  "Recall activity": "召回活动",
  "Recall is issuing": "召回正在以每小时",
  "Recent active Pages": "近期活跃页面",
  "Request activity": "请求活动",
  "read calls per search. Check Access if the client should be idle.": "次读取调用/搜索。若客户端本应空闲，请检查访问审计。",
  "Retrieval volume and reach, not result quality.": "检索量与覆盖范围，不代表结果质量。",
  "Revision history remains protected unless no active dependency needs it. Cleanup remains an explicit local operator action.": "历史版本会持续受到保护，除非已没有任何活跃依赖需要它。实际回收仍是显式的本地操作。",
  "Revision storage": "修订存储",
  "Revision storage preview": "修订存储预览",
  "Review signals": "复核信号",
  "Runtime": "运行时",
  "Runtime activity": "运行时活动",
  "Runtime failures and authorization denials": "运行时失败和授权拒绝",
  "Runtime metrics use operation metadata. They do not evaluate whether returned content is relevant.": "运行时指标使用操作元数据，不评估返回内容是否相关。",
  "Runtime service": "运行时服务",
  "search": "搜索",
  "search/read calls per hour at": "次搜索/读取调用，平均每次搜索",
  "scope": "范围",
  "scopes": "范围",
  "shown of": "项，共",
  "Summary / detail reads": "摘要 / 详情读取",
  "Telemetry coverage": "遥测覆盖率",
  "This view computes a plan only. No revision is deleted.": "本视图只计算规划，不会删除任何修订。",
  "This check is read-only. No revision is deleted from the Console.": "此检查为只读；控制台不会删除任何修订。",
  "Storage safety": "存储安全",
  "Next phase": "下一阶段",
  "Run Scan to discover maintenance work": "先扫描以发现维护工作",
  "Run Scan to inspect this phase": "先扫描以检查此阶段",
  "work items": "个工作项",
  "estimated model calls": "预计模型调用",
  "groups": "组",
  "Extend Pack": "扩展打包页",
  "Merge Packs": "合并打包页",
  "entries": "条目",
  "total": "项",
  "Update preview": "更新预览",
  "window": "时间范围",
  "Why history remains protected": "为什么历史仍受保护",
  "Zero-result": "零结果",
  "Age cannot be proven safely": "无法安全证明其年龄",
  "Created less than": "创建于不足",
  "Current head": "当前头版本",
  "day ago": "天前",
  "days ago": "天前",
  "Exact version recorded at a Relation endpoint": "记录在关联端点上的精确版本",
  "Explicit retention lease": "显式保留租约",
  "Held by a finite retention lease": "受有限期保留租约持有",
  "Idempotency window": "幂等窗口",
  "Immutable source evidence": "不可变的源证据",
  "Invalid timestamp": "无效时间戳",
  "Minimum age window": "最小年龄窗口",
  "Needed to replay a recent write safely": "安全重放近期写入所需",
  "Newest": "最新",
  "Projection head": "投影头版本",
  "Provenance dependency": "溯源依赖",
  "Recent Revision window": "近期修订窗口",
  "Referenced by a Summary record": "被摘要记录引用",
  "Referenced by a Validity assessment": "被有效性评估引用",
  "Relation basis": "关联依据",
  "Relation endpoint": "关联端点",
  "Revision on each Page": "个页面修订",
  "Revisions on each Page": "个页面修订",
  "Sealed evidence": "已封存证据",
  "Store-defined protection root": "Store 定义的保护根",
  "Summary record": "摘要记录",
  "The Page version used by default reads": "默认读取所使用的页面版本",
  "Validity record": "有效性记录",
  "Current Summary or Validity projection": "当前摘要或有效性投影",
  "Evidence used to assert a Relation": "用于断言关联的证据",
};

let themeSetting = "system";
let languageSetting = "system";
let pageLimit = DEFAULT_PAGE_LIMIT;
let activePreferencesTab = "general";
let currentLanguage = "en";

function readPreference(key, fallback) {
  try { return window.localStorage.getItem(key) || fallback; } catch (_) { return fallback; }
}

function writePreference(key, value) {
  try { window.localStorage.setItem(key, value); } catch (_) {}
}

function normalizedPageLimit(value) {
  const limit = Number(value);
  return PAGE_LIMIT_OPTIONS.has(limit) ? limit : DEFAULT_PAGE_LIMIT;
}

function setPageLimit(value) {
  pageLimit = normalizedPageLimit(value);
  writePreference(PAGE_LIMIT_STORAGE_KEY, String(pageLimit));
  byId("page-limit-setting").value = String(pageLimit);
  resetPages();
  if (state.pages.loaded) loadPages().catch(showError);
}

function resolvedLanguage() {
  if (languageSetting === "zh" || languageSetting === "en") return languageSetting;
  return navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}

function t(value) {
  return currentLanguage === "zh" ? (ZH_MESSAGES[value] || value) : value;
}

const PAGE_ORDER_LABELS = {
  recent: "Recent activity",
  oldest: "Oldest first",
  most_connected: "Most direct links",
  least_connected: "Fewest direct links",
  largest: "Largest content",
  source_order: "Source order",
};

function pageOrderLabel(value) {
  return t(PAGE_ORDER_LABELS[value] || PAGE_ORDER_LABELS.recent);
}

function pageOrderValue() {
  const key = state.pages.sortKey;
  const descending = byId("page-sort-direction").dataset.direction === "descending";
  if (key === "connections") return descending ? "most_connected" : "least_connected";
  return descending ? "recent" : "oldest";
}

function pageMenuChoice(label, selected, dataset = {}) {
  const choice = element("button", `page-menu-choice${selected ? " selected" : ""}`);
  choice.type = "button";
  Object.entries(dataset).forEach(([key, value]) => { choice.dataset[key] = value; });
  choice.append(element("span", "page-menu-check", selected ? "✓" : ""), element("span", "", label));
  return choice;
}

function renderPageScopeOptions(scopes = state.overview?.scopes || []) {
  const options = [{ namespace: "", displayName: t("All scopes") }, ...scopes];
  byId("page-filter-options").replaceChildren(...options.map((scope) => pageMenuChoice(
    scope.displayName || scope.namespace,
    scope.namespace === state.pages.scope,
    { pageScope: scope.namespace },
  )));
}

function renderPageSortOptions() {
  const options = [["activity", "Activity time"], ["connections", "Direct links"]];
  byId("page-sort-options-list").replaceChildren(...options.map(([key, label]) => pageMenuChoice(
    t(label),
    key === state.pages.sortKey,
    { pageSortKey: key },
  )));
}

function setPageSortDirection(descending) {
  const control = byId("page-sort-direction");
  const label = t(descending ? "Descending" : "Ascending");
  control.dataset.direction = descending ? "descending" : "ascending";
  control.title = label;
  control.setAttribute("aria-label", label);
}

function closePageMenus() {
  for (const [triggerId, menuId] of [["page-filter-toggle", "page-filter-menu"], ["page-sort-options", "page-sort-menu"]]) {
    byId(menuId).hidden = true;
    byId(triggerId).setAttribute("aria-expanded", "false");
  }
}

function togglePageMenu(triggerId, menuId) {
  const menu = byId(menuId);
  const opening = menu.hidden;
  closePageMenus();
  menu.hidden = !opening;
  byId(triggerId).setAttribute("aria-expanded", String(opening));
}

function currentLocale() {
  return currentLanguage === "zh" ? "zh-CN" : "en-US";
}

function applyStaticTranslations() {
  document.documentElement.lang = currentLanguage === "zh" ? "zh-CN" : "en";
  for (const node of document.querySelectorAll("[data-i18n]")) node.textContent = t(node.dataset.i18n);
  for (const node of document.querySelectorAll("[data-i18n-placeholder]")) node.placeholder = t(node.dataset.i18nPlaceholder);
  for (const node of document.querySelectorAll("[data-i18n-title]")) node.title = t(node.dataset.i18nTitle);
  for (const node of document.querySelectorAll("[data-i18n-aria-label]")) node.setAttribute("aria-label", t(node.dataset.i18nAriaLabel));
}

function applyPreferences() {
  if (themeSetting === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.dataset.theme = themeSetting;
  currentLanguage = resolvedLanguage();
  applyStaticTranslations();
  document.querySelectorAll("[data-theme-setting]").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.themeSetting === themeSetting));
  });
  document.querySelectorAll("[data-language-setting]").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.languageSetting === languageSetting));
  });
}

function initializeConsolePreferences() {
  themeSetting = readPreference(THEME_STORAGE_KEY, "system");
  languageSetting = readPreference(LANGUAGE_STORAGE_KEY, "system");
  pageLimit = normalizedPageLimit(readPreference(PAGE_LIMIT_STORAGE_KEY, String(DEFAULT_PAGE_LIMIT)));
  byId("page-limit-setting").value = String(pageLimit);
  applyPreferences();
  byId("preferences-open").addEventListener("click", () => openPreferences().catch(showError));
  byId("preferences-close").addEventListener("click", () => byId("preferences-dialog").close());
  document.querySelectorAll("[data-preferences-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      activePreferencesTab = button.dataset.preferencesTab;
      renderPreferencesTabs();
    });
  });
  document.querySelectorAll("[data-theme-setting]").forEach((button) => {
    button.addEventListener("click", () => {
      themeSetting = button.dataset.themeSetting;
      writePreference(THEME_STORAGE_KEY, themeSetting);
      applyPreferences();
    });
  });
  document.querySelectorAll("[data-language-setting]").forEach((button) => {
    button.addEventListener("click", () => {
      const wasLanguage = currentLanguage;
      languageSetting = button.dataset.languageSetting;
      writePreference(LANGUAGE_STORAGE_KEY, languageSetting);
      applyPreferences();
      if (currentLanguage !== wasLanguage) rerenderForLocale();
    });
  });
  byId("page-limit-setting").addEventListener("change", (event) => setPageLimit(event.target.value));
  window.addEventListener("languagechange", () => {
    if (languageSetting === "system") {
      const wasLanguage = currentLanguage;
      applyPreferences();
      if (currentLanguage !== wasLanguage) rerenderForLocale();
    }
  });
}

const byId = (id) => document.getElementById(id);
const PROJECTION_TRUNCATION_MARKER = "[projection truncated by host budget]";
const PREVIEW_FALLBACK_CONCURRENCY = 4;
initializeConsolePreferences();
const state = {
  overview: null,
  activeView: "overview",
  pages: {
    loaded: false,
    busy: false,
    scope: "",
    sortKey: "activity",
    count: 0,
    total: 0,
    page: 1,
    pageCache: new Map(),
    cursors: new Map([[1, null]]),
    previewFallbacks: new Map(),
    previewGeneration: 0,
  },
  maintenance: {
    loaded: false,
    busy: false,
    activity: null,
    status: null,
    session: {
      state: "idle",
      startedAt: null,
      completedAt: null,
      outcomes: {
        pack: null,
        summary: null,
        relation: null,
      },
    },
    scan: null,
    pass: "pack",
    workflowStage: "scan",
    phase: "pack",
    analyses: { pack: null, summary: null, relation: null },
    pendingCandidates: [],
    selected: new Set(),
    relationReviews: [],
  },
  access: { loaded: false, busy: false, cursor: null, count: 0, events: [] },
  enrollment: { available: false, seenPending: new Set() },
};
function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function formatNumber(value) {
  return new Intl.NumberFormat(currentLocale()).format(value || 0);
}

function formatSize(chars) {
  if (chars < 1000) return `${chars} chars`;
  if (chars < 1_000_000) return `${(chars / 1000).toFixed(1)}k chars`;
  return `${(chars / 1_000_000).toFixed(2)}M chars`;
}

function formatCandidateGroups(value) {
  const count = Number(value) || 0;
  return `${formatNumber(count)} candidate group${count === 1 ? "" : "s"}`;
}

function formatTime(value) {
  if (!value) return "-";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString(currentLocale());
}

function confirmAction({ title, description, confirmLabel = "Continue" }) {
  const dialog = byId("action-dialog");
  byId("action-dialog-title").textContent = title;
  byId("action-dialog-description").textContent = description;
  byId("action-dialog-confirm").textContent = confirmLabel;
  dialog.returnValue = "cancel";
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "confirm"), { once: true });
    dialog.showModal();
    byId("action-dialog-cancel").focus();
  });
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { Accept: "application/json", ...(options.headers || {}) },
  });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try { message = (await response.json()).error || message; } catch (_) {}
    throw new Error(message);
  }
  return response.status === 204 ? null : response.json();
}

async function enrollmentMutation(path) {
  return api(path, {
    method: "POST",
    headers: { "X-PCP-Console": "1" },
  });
}

async function maintenanceMutation(path, body) {
  return api(path, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-PCP-Console": "1" },
    body: JSON.stringify(body),
  });
}

function enrollmentAccessLabel(access) {
  const scopes = access.scopes.join(", ");
  return `${access.mode} / ${scopes}${access.allow_cross_scope_derivation ? " / cross-scope derivation" : ""}`;
}

function enrollmentIdentity(client) {
  const principal = client.principal;
  return principal.displayName || principal.principalId;
}

function enrollmentRow(item, pending) {
  const row = element("article", "enrollment-row");
  const identity = element("div", "enrollment-identity");
  identity.append(
    element("strong", "", enrollmentIdentity(item.client)),
    element("span", "mono muted", item.client.principal.principalId),
    element("span", "muted", enrollmentAccessLabel(pending ? item.requested_access : item.approved_access)),
  );
  const actions = element("div", "enrollment-actions");
  if (pending) {
    const reject = element("button", "", "Reject");
    reject.type = "button";
    reject.addEventListener("click", async () => {
      reject.disabled = true;
      try {
        await enrollmentMutation(`/api/enrollment/requests/${encodeURIComponent(item.request_id)}/reject`);
        await loadEnrollment({ autoOpen: false });
      } catch (error) {
        reject.disabled = false;
        showError(error);
      }
    });
    const approve = element("button", "primary-button", "Approve");
    approve.type = "button";
    approve.addEventListener("click", async () => {
      approve.disabled = true;
      try {
        await enrollmentMutation(`/api/enrollment/requests/${encodeURIComponent(item.request_id)}/approve`);
        await loadEnrollment({ autoOpen: false });
      } catch (error) {
        approve.disabled = false;
        showError(error);
      }
    });
    actions.append(reject, approve);
  } else {
    const revoke = element("button", "danger-button", "Revoke");
    revoke.type = "button";
    revoke.addEventListener("click", async () => {
      revoke.disabled = true;
      try {
        await enrollmentMutation(`/api/enrollment/registrations/${encodeURIComponent(item.registration_id)}/revoke`);
        await loadEnrollment({ autoOpen: false });
      } catch (error) {
        revoke.disabled = false;
        showError(error);
      }
    });
    actions.append(revoke);
  }
  row.append(identity, actions);
  return row;
}

function renderEnrollment(data, autoOpen) {
  const snapshot = data.result;
  if (snapshot.status !== "snapshot") throw new Error("Unexpected enrollment response");
  state.enrollment.available = true;
  byId("enrollment-open").hidden = false;
  const pending = snapshot.pending || [];
  const registered = snapshot.registrations || [];
  const badge = byId("enrollment-badge");
  badge.textContent = String(pending.length);
  badge.hidden = pending.length === 0;

  const pendingList = byId("enrollment-pending");
  pendingList.replaceChildren(...pending.map((item) => enrollmentRow(item, true)));
  if (pending.length === 0) pendingList.append(element("div", "empty enrollment-empty", "No pending requests"));
  const registeredList = byId("enrollment-registered");
  registeredList.replaceChildren(...registered.map((item) => enrollmentRow(item, false)));
  if (registered.length === 0) registeredList.append(element("div", "empty enrollment-empty", "No approved clients"));

  const unseen = pending.filter((item) => !state.enrollment.seenPending.has(item.request_id));
  if (autoOpen && unseen.length > 0 && !document.querySelector("dialog[open]")) {
    byId("enrollment-dialog").showModal();
  }
  if (byId("enrollment-dialog").open) {
    pending.forEach((item) => state.enrollment.seenPending.add(item.request_id));
  }
}

async function loadEnrollment({ autoOpen = true } = {}) {
  try {
    renderEnrollment(await api("/api/enrollment"), autoOpen);
  } catch (_) {
    if (!state.enrollment.available) byId("enrollment-open").hidden = true;
  }
}

function showError(error) {
  const box = byId("error");
  box.textContent = error.message || String(error);
  box.hidden = false;
  window.setTimeout(() => { box.hidden = true; }, 7000);
}

const pageInspector = createPageInspector({ request: api, showError, formatTime, t });
const queryView = createQueryView({
  request: api,
  byId,
  element,
  showError,
  t,
  formatNumber,
  openPage: (pageId) => pageInspector.open(pageId),
});
const healthView = createHealthView({ request: api, showError, formatNumber, t });
const retentionView = createRetentionView({
  request: api,
  showError,
  formatNumber,
  formatTime,
  openPage: (pageId) => pageInspector.open(pageId),
  t,
});

function metric(label, value, tone = "") {
  const node = element("div", `metric${tone ? ` tone-${tone}` : ""}`);
  node.append(element("div", "metric-label", label), element("div", "metric-value", value));
  return node;
}

function decisionTone(decision) {
  const value = String(decision || "").toLowerCase();
  if (["allow", "allowed", "granted"].includes(value)) return "allowed";
  if (["deny", "denied", "rejected"].includes(value)) return "denied";
  return "other";
}

function scopeName(namespace) {
  const scope = state.overview?.scopes.find((item) => item.namespace === namespace);
  return scope?.displayName || namespace || t("All scopes");
}

function orderedScopes(scopes) {
  const byNamespace = new Map(scopes.map((scope) => [scope.namespace, scope]));
  const children = new Map();
  const roots = [];
  for (const scope of scopes) {
    if (scope.parentNamespace && byNamespace.has(scope.parentNamespace)) {
      const siblings = children.get(scope.parentNamespace) || [];
      siblings.push(scope);
      children.set(scope.parentNamespace, siblings);
    } else {
      roots.push(scope);
    }
  }
  const compare = (left, right) => (left.displayName || left.namespace).localeCompare(right.displayName || right.namespace);
  const output = [];
  const visited = new Set();
  function visit(scope, depth) {
    if (visited.has(scope.namespace)) return;
    visited.add(scope.namespace);
    output.push({ scope, depth });
    (children.get(scope.namespace) || []).sort(compare).forEach((child) => visit(child, depth + 1));
  }
  roots.sort(compare).forEach((scope) => visit(scope, 0));
  scopes.sort(compare).forEach((scope) => visit(scope, 0));
  return output;
}

function renderOverview(data) {
  state.overview = data;
  queryView.setScopes(data.scopes || []);
  const connected = data.integrity === "ok";
  byId("connection").textContent = connected ? t("Connected") : t("Degraded");
  byId("connection").classList.toggle("ready", connected);
  byId("connection").classList.toggle("degraded", !connected);
  byId("headline-pages").textContent = formatNumber(data.pageCount);
  byId("headline-content").textContent = formatSize(data.contentChars);

  byId("metrics").replaceChildren(
    metric(t("Integrity"), data.integrity, connected ? "positive" : "danger"),
    metric(t("Protocol"), data.capabilities.protocolVersion, "info"),
    metric(t("Runtime PID"), data.runtime.pid || "-"),
    metric(t("Runtime started"), formatTime(data.runtime.startedAtUnixMs)),
  );

  byId("scope-rows").replaceChildren(...orderedScopes([...data.scopes]).map(({ scope, depth }) => {
    const row = document.createElement("tr");
    const open = element("button", "quiet-button", t("Open"));
    open.type = "button";
    open.title = `Browse ${scope.displayName || scope.namespace}`;
    open.addEventListener("click", () => openScope(scope.namespace));
    const action = element("td", "action-cell");
    action.append(open);
    const identity = element("td");
    const scopeCell = element("div", "scope-cell");
    scopeCell.style.setProperty("--scope-depth", depth);
    scopeCell.append(
      element("strong", "", scope.displayName || scope.namespace),
      element("span", "mono muted", scope.namespace),
    );
    if (scope.description) scopeCell.append(element("span", "scope-description", scope.description));
    identity.append(scopeCell);
    row.append(
      identity,
      element("td", "", formatNumber(scope.pageCount)),
      element("td", "", formatTime(scope.updatedAt)),
      action,
    );
    return row;
  }));

  const endpointRows = [
    [t("Principal"), data.principal.principalId],
    [t("Principal type"), data.principal.principalType],
    [t("Identity"), data.identityId],
    [t("Session"), data.grants.length ? t("active") : t("no grants")],
    [t("Granted scopes"), data.grants.map((grant) => grant.namespace).join(", ")],
  ];
  byId("endpoint-details").replaceChildren(...endpointRows.flatMap(([label, value]) => [
    element("dt", "", label),
    element("dd", "mono", value),
  ]));

  const capabilities = data.capabilities;
  const features = new Set(capabilities.features || []);
  const capabilityRows = [
    [t("Access audit"), features.has("access_audit")],
    [t("Lossless page packing"), features.has("lossless_page_packing")],
    [t("Retention planning"), features.has("revision_retention_planning")],
    [t("Retention leases"), features.has("revision_retention_leases")],
    [t("Retention collection"), features.has("revision_retention")],
  ];
  byId("capability-details").replaceChildren(...capabilityRows.flatMap(([label, enabled]) => [
    element("dt", "", label),
    element("dd", enabled ? "capability-yes" : "capability-no", enabled ? t("Available") : t("Unavailable")),
  ]));

  renderPageScopeOptions(data.scopes);
  renderPageSortOptions();
}

function aggregatePanel(label, entries) {
  const panel = element("section", "aggregate-panel");
  panel.append(element("h3", "", label));
  for (const [name, count] of entries) {
    const row = element("div", "aggregate-row");
    if (label === "Decisions") row.classList.add(`decision-${decisionTone(name)}`);
    row.append(element("span", "mono", name), element("strong", "", formatNumber(count)));
    panel.append(row);
  }
  if (!entries.length) panel.append(element("div", "empty", t("No events")));
  return panel;
}

function topCounts(events, select) {
  const counts = new Map();
  for (const event of events) {
    const key = select(event) || "unknown";
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  return [...counts.entries()].sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0])).slice(0, 6);
}

function renderAccessSummary() {
  byId("access-summary").replaceChildren(
    aggregatePanel(t("Operations"), topCounts(state.access.events, (event) => event.operation)),
    aggregatePanel(t("Principals"), topCounts(state.access.events, (event) => event.principal.principalId)),
    aggregatePanel(t("Decisions"), topCounts(state.access.events, (event) => event.decision)),
  );
}

function pageResult(hit) {
  const result = element("article", "page-result");
  result.dataset.pageId = hit.pageId;
  const open = element("button", "page-link");
  open.type = "button";
  open.append(
    element("strong", "page-title", pageSnippet(hit)),
  );
  open.addEventListener("click", () => pageInspector.open(hit.pageId));
  result.append(open, pageResultMeta(hit));
  return result;
}

function pageSnippet(hit) {
  if (hit.matchedProjection === "summary" || hit.matchedProjection === "facets") {
    return hit.snippet || t("No preview");
  }
  const payload = hit.previewPayload;
  if (hasBudgetTruncatedProjection(payload)) {
    return hit.snippet || t("Preview unavailable");
  }
  const presentation = describePagePayload(payload?.content, payload?.mediaType);
  if (presentation.type === "external_signal") {
    return presentation.title || presentation.summary || presentation.content || t("Signal");
  }
  if (presentation.type === "image_asset") return presentation.filename || t("Image");
  if (presentation.type === "packed_page" && presentation.entries.length) {
    return presentation.entries.find((entry) => entry.role === "user")?.content
      || presentation.entries[0].content
      || t("Conversation pack");
  }
  const preview = pagePayloadPreviewText(payload?.content, payload?.mediaType);
  if (payload?.mediaType === "application/vnd.pcp.packed-page+json" && preview.trimStart().startsWith("{")) {
    return hit.sourceSpan
      ? `${t("Source positions")} ${hit.sourceSpan.start}–${hit.sourceSpan.end}`
      : t("Conversation pack");
  }
  return preview || hit.snippet || t("No preview");
}

function hasBudgetTruncatedProjection(payload) {
  return typeof payload?.content === "string"
    && payload.content.trimEnd().endsWith(PROJECTION_TRUNCATION_MARKER);
}

function renderPageRowPreview(row, hit) {
  const title = row.querySelector(".page-title");
  if (title) title.textContent = pageSnippet(hit);
}

async function refreshTruncatedPagePreviews(entries, generation) {
  const pending = entries.filter(([hit]) => (
    hasBudgetTruncatedProjection(hit.previewPayload)
      && !state.pages.previewFallbacks.has(hit.pageId)
  ));
  let next = 0;
  const worker = async () => {
    while (next < pending.length) {
      const [hit, row] = pending[next++];
      try {
        const preview = await api(`/api/pages/${encodeURIComponent(hit.pageId)}/preview`);
        if (!preview.previewPayload || hasBudgetTruncatedProjection(preview.previewPayload)) continue;
        state.pages.previewFallbacks.set(hit.pageId, preview.previewPayload);
        hit.previewPayload = preview.previewPayload;
        if (state.pages.previewGeneration === generation && row.isConnected) {
          renderPageRowPreview(row, hit);
        }
      } catch (_) {
        // Background repair is opportunistic; leave the neutral list preview on failure.
      }
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(PREVIEW_FALLBACK_CONCURRENCY, pending.length) }, worker),
  );
}

function pageSourceLabel(hit) {
  let source = hit.namespace;
  if (hit.sourceSpan) {
    source += ` · ${t("Source positions")} ${hit.sourceSpan.start}–${hit.sourceSpan.end}`;
  }
  return source;
}

function pageStructureTags(hit) {
  const tags = [];
  if (hit.sourceSpan) tags.push(["⌁", t("Source stream")]);
  if (hit.summaryRevisionId) tags.push(["≡", t("Summary route")]);
  if (hit.previewPayload?.mediaType === "application/vnd.pcp.packed-page+json") tags.push(["▣", t("Packed")]);
  return tags;
}

function pageRelationSignal(hit) {
  const stats = hit.relationStats;
  const signal = element("span", `page-signal${stats?.total ? "" : " page-signal-empty"}`);
  if (!stats) {
    signal.title = t("Unavailable");
    signal.append(element("span", "page-signal-icon", "↔"), element("span", "", "–"));
    return signal;
  }
  signal.title = stats.total > 0
    ? `${formatNumber(stats.total)} ${t("Direct links")} · ${formatNumber(stats.incoming)} ${t("in")} · ${formatNumber(stats.outgoing)} ${t("out")}`
    : t("No direct links");
  signal.append(
    element("span", "page-signal-icon", "↔"),
    element("strong", "", formatNumber(stats.total)),
    element("span", "page-signal-direction", `↓ ${formatNumber(stats.incoming)}`),
    element("span", "page-signal-direction", `↑ ${formatNumber(stats.outgoing)}`),
  );
  return signal;
}

function pageResultMeta(hit) {
  const meta = element("div", "page-result-meta");
  const entries = [
    [t("Kind"), hit.kind],
    [t("Source"), pageSourceLabel(hit)],
    [t("Observed"), formatTime(hit.observedAt || hit.createdAt)],
  ];
  meta.append(...entries.map(([label, value]) => {
    const item = element("span", "page-meta-item");
    item.append(element("span", "page-meta-label", label), document.createTextNode(value));
    return item;
  }));
  meta.append(pageRelationSignal(hit));
  meta.append(...pageStructureTags(hit).map(([icon, label]) => {
    const tag = element("span", "page-structure-tag");
    tag.title = label;
    tag.append(element("span", "", icon), document.createTextNode(label));
    return tag;
  }));
  return meta;
}

function renderPages(data, page) {
  const results = byId("page-results");
  for (const hit of data.hits) {
    const cachedPreview = state.pages.previewFallbacks.get(hit.pageId);
    if (cachedPreview) hit.previewPayload = cachedPreview;
  }
  const rendered = data.hits.map((hit) => [hit, pageResult(hit)]);
  state.pages.page = page;
  state.pages.count = data.hits.length;
  state.pages.total = Number.isFinite(data.totalPages) ? data.totalPages : state.pages.count;
  state.pages.pageCache.set(page, data);
  state.pages.cursors.set(page + 1, data.nextCursor || null);
  state.pages.loaded = true;
  byId("pages-status").textContent = `${scopeName(state.pages.scope)} · ${pageOrderLabel(pageOrderValue())}`;
  const pageCount = Math.max(1, Math.ceil(state.pages.total / pageLimit));
  byId("pages-loaded").textContent = `${formatNumber(state.pages.total)} ${t("Pages")}`;
  byId("pages-current").textContent = `${t("Page number")} ${formatNumber(page)} / ${formatNumber(pageCount)} ${t("Pages")}`;
  byId("pages-pager").hidden = pageCount <= 1;
  byId("pages-previous").disabled = page <= 1;
  byId("pages-next").disabled = !data.nextCursor;

  if (state.pages.count === 0) {
    results.replaceChildren(element("div", "empty", t("No pages")));
  } else {
    results.replaceChildren(...rendered.map(([, result]) => result));
  }

  void refreshTruncatedPagePreviews(rendered, state.pages.previewGeneration);
}

function renderAccess(data, append) {
  const rows = byId("access-rows");
  const rendered = data.events.map((event) => {
    const row = document.createElement("tr");
    row.append(
      element("td", "", formatTime(event.occurredAt)),
      element("td", "mono", event.principal.principalId),
      element("td", "mono", event.operation),
      element("td", "mono", event.scopes.join(", ")),
      element("td", `decision-${decisionTone(event.decision)}`, event.decision),
    );
    return row;
  });
  if (append) rows.append(...rendered);
  else rows.replaceChildren(...rendered);
  state.access.events = append ? state.access.events.concat(data.events) : data.events;
  state.access.count = append ? state.access.count + data.events.length : data.events.length;
  state.access.cursor = data.nextCursor || null;
  state.access.loaded = true;
  byId("access-status").textContent = t("Audit timeline");
  byId("access-loaded").textContent = `${formatNumber(state.access.count)} ${t("loaded")}`;
  byId("access-more").hidden = !state.access.cursor;
  renderAccessSummary();
}

async function loadOverview() {
  renderOverview(await api("/api/overview"));
}

function resetPages() {
  state.pages.page = 1;
  state.pages.count = 0;
  state.pages.total = 0;
  state.pages.pageCache.clear();
  state.pages.cursors = new Map([[1, null]]);
  state.pages.previewGeneration += 1;
}

async function loadPages({ page = state.pages.page } = {}) {
  if (state.pages.busy) return;
  const cached = state.pages.pageCache.get(page);
  if (cached) {
    renderPages(cached, page);
    return;
  }
  const cursor = state.pages.cursors.get(page);
  if (page > 1 && !cursor) return;
  state.pages.busy = true;
  byId("pages-status").textContent = t("Loading");
  byId("pages-previous").disabled = true;
  byId("pages-next").disabled = true;
  try {
    const params = new URLSearchParams({ limit: String(pageLimit) });
    const query = byId("query").value.trim();
    const scope = state.pages.scope;
    if (query) {
      params.set("q", query);
    }
    params.set("order", pageOrderValue());
    if (scope) params.set("scope", scope);
    if (cursor) params.set("cursor", cursor);
    renderPages(await api(`/api/pages?${params}`), page);
  } catch (error) {
    showError(error);
    byId("pages-status").textContent = t("Load failed");
  } finally {
    state.pages.busy = false;
    const current = state.pages.pageCache.get(state.pages.page);
    byId("pages-previous").disabled = state.pages.page <= 1;
    byId("pages-next").disabled = !current?.nextCursor;
  }
}

async function loadAccess({ append = false } = {}) {
  if (state.access.busy) return;
  state.access.busy = true;
  byId("access-status").textContent = append ? t("Loading more") : t("Loading");
  byId("access-more").disabled = true;
  try {
    const params = new URLSearchParams({ limit: String(ACCESS_LIMIT) });
    if (append && state.access.cursor) params.set("cursor", state.access.cursor);
    renderAccess(await api(`/api/access?${params}`), append);
  } catch (error) {
    showError(error);
    byId("access-status").textContent = t("Load failed");
  } finally {
    state.access.busy = false;
    byId("access-more").disabled = false;
  }
}

function emptyMaintenanceOutcome() {
  return {
    scannedAt: null,
    analyzedAt: null,
    workItems: 0,
    modelCalls: 0,
    proposals: 0,
    applied: 0,
    skipped: 0,
    completed: false,
    issues: [],
  };
}

function resetMaintenanceSession() {
  state.maintenance.session = {
    state: "idle",
    startedAt: null,
    completedAt: null,
    outcomes: {
      pack: emptyMaintenanceOutcome(),
      summary: emptyMaintenanceOutcome(),
      relation: emptyMaintenanceOutcome(),
    },
  };
  state.maintenance.pass = "pack";
  state.maintenance.workflowStage = "scan";
  state.maintenance.phase = "pack";
  state.maintenance.scan = null;
  state.maintenance.analyses = { pack: null, summary: null, relation: null };
  state.maintenance.pendingCandidates = [];
  state.maintenance.selected.clear();
}

function renderMaintenanceStatus(status) {
  state.maintenance.status = status;
  state.maintenance.loaded = true;
  renderAutomationStatus();
  renderMaintenanceSession();
}

function automationStateLabel(status) {
  if (!status.enabled) return t("Disabled");
  const labels = {
    not_started: "Not started",
    waiting: "Waiting",
    running: "Running",
    failed: "Failed",
    stale: "Stale",
  };
  return t(labels[status.automation?.state] || "Not started");
}

function automationStateTone(status) {
  if (!status.enabled || status.automation?.state === "not_started") return "warning";
  if (["failed", "stale"].includes(status.automation?.state)) return "danger";
  if (status.automation?.state === "running") return "info";
  return "positive";
}

function renderAutomationStatus() {
  const status = state.maintenance.status;
  const section = byId("maintenance-automation-status");
  section.hidden = !status?.available;
  if (section.hidden) return;
  const automation = status.automation || {};
  const trigger = status.writeTrigger || {};
  const stateNode = byId("maintenance-automation-state");
  stateNode.textContent = automationStateLabel(status);
  stateNode.className = `status-pill status-${automationStateTone(status)}`;
  byId("maintenance-automation-metrics").replaceChildren(
    metric(t("Observed heads"), formatNumber(automation.observedPageCount)),
    metric(t("Dirty regions"), formatNumber(automation.dirtyRegionCount), automation.dirtyRegionCount ? "warning" : ""),
    metric(t("Ready regions"), formatNumber(automation.readyRegionCount), automation.readyRegionCount ? "info" : ""),
    metric(t("Pending relation review"), formatNumber(automation.pendingRelationReviewCount), automation.pendingRelationReviewCount ? "warning" : ""),
    metric(t("Write trigger"), `${formatNumber(trigger.minNewPages)} / ${Math.round((trigger.quietPeriodSeconds || 0) / 60)}m / ${Math.round((trigger.maxWaitSeconds || 0) / 60)}m`),
  );
  const completed = automation.lastCompletedAt;
  byId("maintenance-automation-detail").textContent = completed
    ? `${t("Last completed")}: ${formatTime(completed)}`
    : t("Awaiting the first Runtime heartbeat.");
  const error = byId("maintenance-automation-error");
  error.hidden = !automation.lastError;
  error.textContent = automation.lastError || "";
}

function populateMaintenanceSettings() {
  const status = state.maintenance.status;
  const configurable = Boolean(status?.configurable);
  byId("maintenance-settings-section").hidden = !configurable;
  byId("maintenance-settings-tab").hidden = !configurable;
  if (!configurable && activePreferencesTab === "maintenance") activePreferencesTab = "general";
  if (!configurable) return;
  const trigger = status.writeTrigger || {};
  byId("maintenance-settings-enabled").checked = Boolean(status.enabled);
  byId("maintenance-settings-mode").value = status.mode || "observe";
  byId("maintenance-settings-min-pages").value = trigger.minNewPages || 8;
  byId("maintenance-settings-quiet").value = Math.max(1, Math.round((trigger.quietPeriodSeconds || 600) / 60));
  byId("maintenance-settings-max-wait").value = Math.max(1, Math.round((trigger.maxWaitSeconds || 3600) / 60));
}

function renderPreferencesTabs() {
  document.querySelectorAll("[data-preferences-tab]").forEach((button) => {
    const active = button.dataset.preferencesTab === activePreferencesTab && !button.hidden;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });
  document.querySelectorAll("[data-preferences-panel]").forEach((panel) => {
    panel.classList.toggle("active", panel.dataset.preferencesPanel === activePreferencesTab && !panel.hidden);
  });
}

async function openPreferences() {
  if (!state.maintenance.loaded) renderMaintenanceStatus(await api("/api/maintenance"));
  populateMaintenanceSettings();
  activePreferencesTab = "general";
  renderPreferencesTabs();
  byId("preferences-dialog").showModal();
}

async function saveMaintenanceSettings(event) {
  event.preventDefault();
  const minNewPages = Number(byId("maintenance-settings-min-pages").value);
  const quietPeriodSeconds = Number(byId("maintenance-settings-quiet").value) * 60;
  const maxWaitSeconds = Number(byId("maintenance-settings-max-wait").value) * 60;
  if (!Number.isInteger(minNewPages) || minNewPages < 1 || !Number.isInteger(quietPeriodSeconds) || quietPeriodSeconds < 60 || !Number.isInteger(maxWaitSeconds) || maxWaitSeconds < quietPeriodSeconds) {
    showError(new Error(t("Maximum wait must be at least the quiet period, and all values must be positive.")));
    return;
  }
  const save = byId("maintenance-settings-save");
  save.disabled = true;
  try {
    await maintenanceMutation("/api/maintenance/settings", {
      enabled: byId("maintenance-settings-enabled").checked,
      mode: byId("maintenance-settings-mode").value,
      minNewPages,
      quietPeriodSeconds,
      maxWaitSeconds,
    });
    byId("preferences-dialog").close();
    await refresh();
  } catch (error) {
    showError(error);
  } finally {
    save.disabled = false;
  }
}

function compactRelationReviewPreview(value, limit = 220) {
  const normalized = String(value || "").replace(/\s+/g, " ").trim();
  return normalized.length <= limit ? normalized : `${normalized.slice(0, limit - 1)}…`;
}

function revisionBoundPageExpansion(page) {
  const fullPage = element("details", "maintenance-relation-review-full-page");
  const fullPageSummary = element("summary", "", t("Expand full Page"));
  const fullPageBody = element("div", "maintenance-relation-review-full-page-body muted", "");
  fullPage.append(fullPageSummary, fullPageBody);
  let loaded = false;
  fullPage.addEventListener("toggle", async () => {
    if (!fullPage.open || loaded) return;
    loaded = true;
    fullPageBody.textContent = t("Loading full Page…");
    try {
      const detail = await api(`/api/pages/${encodeURIComponent(page.revisionId)}`);
      if (detail.revision?.revisionId !== page.revisionId) {
        throw new Error("The reviewed revision is no longer available.");
      }
      fullPageBody.classList.remove("muted");
      fullPageBody.replaceChildren();
      const content = element("div", "page-content");
      renderPagePreview(
        content,
        detail.revision?.payload?.content || t("No content projection"),
        detail.revision?.payload?.mediaType || "text/plain",
      );
      const open = element("button", "subtle-button", t("Open in inspector"));
      open.type = "button";
      open.addEventListener("click", () => pageInspector.open(page.pageId));
      fullPageBody.append(content, open);
    } catch (error) {
      loaded = false;
      fullPageBody.textContent = error.message || String(error);
      showError(error);
    }
  });
  return fullPage;
}

function relationReviewCard(proposal) {
  const card = element("article", "maintenance-relation-review-card");
  const heading = element("div", "maintenance-relation-review-card-heading");
  heading.append(
    element("strong", "", t("Review evidence")),
    element("span", "maintenance-relation-review-risk", t("Manual approval required")),
    element("span", "muted", formatTime(proposal.proposedAt)),
  );
  if (proposal.reviewReason) {
    card.append(element("p", "maintenance-relation-review-reason muted", proposal.reviewReason));
  }
  const pages = element("div", "maintenance-relation-review-pages");
  proposal.pages.forEach((page, index) => {
    const pageColumn = element("div", "maintenance-relation-review-page-column");
    const pageCard = element("button", "maintenance-relation-review-page");
    pageCard.type = "button";
    pageCard.title = page.pageId;
    pageCard.append(
      element("span", "mono muted", page.pageId),
      element("span", "maintenance-relation-review-preview", compactRelationReviewPreview(page.preview)),
      element("span", "mono muted", page.revisionId),
    );
    pageCard.addEventListener("click", () => pageInspector.open(page.pageId));
    pageColumn.append(pageCard, revisionBoundPageExpansion(page));
    pages.append(pageColumn);
    if (index === 0) pages.append(element("span", "maintenance-relation-review-link", "↔"));
  });
  const actions = element("div", "maintenance-relation-review-actions");
  const approve = element("button", "primary-button", t("Approve"));
  const reject = element("button", "", t("Reject"));
  const suppress = element("button", "danger-button", t("Suppress"));
  [approve, reject, suppress].forEach((button) => { button.type = "button"; });
  approve.addEventListener("click", () => resolveRelationReview(proposal.candidateId, "approve"));
  reject.addEventListener("click", () => resolveRelationReview(proposal.candidateId, "reject"));
  suppress.addEventListener("click", () => resolveRelationReview(proposal.candidateId, "suppress"));
  actions.append(approve, reject, suppress);
  card.append(heading, pages, actions);
  return card;
}

function renderRelationReviews() {
  const section = byId("maintenance-relation-review");
  const reviews = state.maintenance.relationReviews;
  section.hidden = !reviews.length;
  byId("maintenance-relation-review-count").textContent = reviews.length
    ? `${formatNumber(reviews.length)} ${t("proposals")}`
    : "";
  byId("maintenance-relation-review-cards").replaceChildren(
    ...reviews.map(relationReviewCard),
  );
}

async function loadRelationReviews() {
  if (!maintenanceAvailable()) {
    state.maintenance.relationReviews = [];
    renderRelationReviews();
    return;
  }
  const response = await api("/api/maintenance/relation-reviews");
  state.maintenance.relationReviews = response.proposals || [];
  renderRelationReviews();
}

async function resolveRelationReview(candidateId, decision) {
  if (state.maintenance.busy) return;
  state.maintenance.busy = true;
  try {
    const suffix = decision === "approve" ? "approve" : "reject";
    await maintenanceMutation(
      `/api/maintenance/relation-reviews/${encodeURIComponent(candidateId)}/${suffix}`,
      decision === "approve" ? {} : { suppress: decision === "suppress" },
    );
    await loadMaintenance({ reload: true });
  } catch (error) {
    showError(error);
  } finally {
    state.maintenance.busy = false;
    renderRelationReviews();
  }
}

function maintenanceAvailable() {
  return Boolean(state.maintenance.status?.available);
}

const MAINTENANCE_PHASES = {
  pack: { label: "Pack", scanKey: "packing", operation: "pack", next: "summary", order: 1 },
  summary: { label: "Summary", scanKey: "summary", operation: "summary", next: "relation", order: 2 },
  relation: { label: "Relations", scanKey: "relation", operation: "relation", next: null, order: 3 },
};

const MAINTENANCE_STAGES = {
  scan: { label: "Scan candidates", order: 1 },
  analyze: { label: "Analyze suggestions", order: 2 },
  review: { label: "Review and apply", order: 3 },
};

const MAINTENANCE_PASSES = {
  pack: { label: "Pack maintenance", phases: ["pack"], order: 1 },
  semantic: { label: "Semantic maintenance", phases: ["summary", "relation"], order: 2 },
};

function maintenancePass() {
  return state.maintenance.pass;
}

function maintenancePassConfig(pass = maintenancePass()) {
  return MAINTENANCE_PASSES[pass];
}

function maintenancePassPhases(pass = maintenancePass()) {
  return maintenancePassConfig(pass).phases;
}

function maintenanceWorkflowStage() {
  return state.maintenance.workflowStage;
}

function maintenanceStageConfig(stage = maintenanceWorkflowStage()) {
  return MAINTENANCE_STAGES[stage];
}

function maintenancePhase() {
  return state.maintenance.phase;
}

function maintenancePhaseConfig(phase = maintenancePhase()) {
  return MAINTENANCE_PHASES[phase];
}

function maintenanceSessionActive() {
  return state.maintenance.session.state === "active";
}

function maintenanceSessionComplete() {
  return state.maintenance.session.state === "complete";
}

function maintenanceOutcome(phase = maintenancePhase()) {
  return state.maintenance.session.outcomes[phase];
}

function maintenanceScanForPhase(phase = maintenancePhase()) {
  const config = maintenancePhaseConfig(phase);
  return state.maintenance.scan?.[config.scanKey] || null;
}

function maintenanceWorkCount(phase = maintenancePhase()) {
  const scan = maintenanceScanForPhase(phase);
  if (!scan) return 0;
  if (phase === "summary") return scan.eligiblePages || 0;
  return scan.candidateGroupCount || 0;
}

function maintenanceWorkLabel(phase = maintenancePhase()) {
  return phase === "summary" ? t("Scanned eligible pages") : t("Scanned candidate groups");
}

function maintenanceEstimatedCalls(phase = maintenancePhase()) {
  return maintenanceScanForPhase(phase)?.estimatedModelCalls || 0;
}

function maintenanceCandidates() {
  if (maintenanceWorkflowStage() === "review") {
    return state.maintenance.pendingCandidates;
  }
  const phase = maintenancePhase();
  if (phase === "pack") {
    return state.maintenance.pendingCandidates
      .filter((candidate) => candidate.operation === "pack");
  }
  return state.maintenance.pendingCandidates
    .filter((candidate) => candidate.operation === phase);
}

function maintenanceSelectionCheckbox(candidate) {
  const checkbox = document.createElement("input");
  checkbox.type = "checkbox";
  checkbox.checked = state.maintenance.selected.has(candidate.candidateId);
  checkbox.setAttribute("aria-label", `Select ${candidate.candidateId}`);
  checkbox.addEventListener("change", () => {
    if (checkbox.checked) state.maintenance.selected.add(candidate.candidateId);
    else state.maintenance.selected.delete(candidate.candidateId);
    renderMaintenanceSession();
  });
  return checkbox;
}

function maintenanceCandidateRow(candidate) {
  const row = document.createElement("tr");
  const selectCell = element("td", "maintenance-select");
  selectCell.append(maintenanceSelectionCheckbox(candidate));

  const source = element("td", "maintenance-source");
  const change = element("td");
  const inputs = element("td", "maintenance-inputs");
  const content = element("td");
  const compactPreview = (value, limit = 180) => {
    const normalized = String(value || "").replace(/\s+/g, " ").trim();
    return normalized.length <= limit ? normalized : `${normalized.slice(0, limit - 1)}…`;
  };
  if (candidate.operation === "summary") {
    source.append(
      element("strong", "", candidate.namespace),
      element("span", "mono muted", candidate.pageId),
    );
    change.append(
      element("strong", "", t("Summary route")),
      element("span", "muted", `${formatSize(candidate.contentChars)} ${t("Page")}`),
    );
    const item = element("div", "maintenance-input");
    item.append(
      element("span", "mono muted", t("Page")),
      element("span", "maintenance-preview", candidate.pageId),
    );
    inputs.append(item);
    content.append(element("span", "maintenance-preview", compactPreview(candidate.content, 240)));
  } else if (candidate.operation === "relation") {
    source.append(
      element("strong", "", candidate.namespace),
      element("span", "muted", `2 ${t("Pages")}`),
    );
    change.append(
      element("strong", "", t("Explicit relation")),
      element("span", "muted", "related_to"),
    );
    for (const page of candidate.pages) {
      const item = element("div", "maintenance-input");
      item.append(
        element("span", "mono muted", page.pageId),
        element("span", "maintenance-preview", compactPreview(page.preview || t("No preview"))),
        revisionBoundPageExpansion(page),
      );
      inputs.append(item);
    }
    content.append(element("strong", "", "related_to"));
  } else {
    const mergesPacks = candidate.pages.length === 2
      && candidate.pages.every((page) => page.mediaType === "application/vnd.pcp.packed-page+json");
    source.append(
      element("strong", "", candidate.namespace),
      element("span", "mono muted", `${t("Stream")} ${candidate.sourceSpan.start}–${candidate.sourceSpan.end}`),
    );
    change.append(
      element("strong", "", t(mergesPacks ? "Merge Packs" : candidate.extendsExistingPack ? "Extend Pack" : "New Pack")),
      element("span", "muted", `${formatNumber(candidate.inputPageCount)} ${t("Pages")} → ${formatNumber(candidate.resultingEntryCount)} ${t("entries")}`),
    );
    for (const page of candidate.pages) {
      const item = element("div", "maintenance-input");
      item.append(
        element("span", "mono muted", `${page.sourceSpan.start}–${page.sourceSpan.end}`),
        element("span", "maintenance-preview", compactPreview(page.preview || page.pageId)),
      );
      inputs.append(item);
    }
    content.append(element("strong", "", formatSize(candidate.contentChars)));
  }
  row.append(selectCell, source, change, inputs, content);
  return row;
}

function renderMaintenanceCandidateRows(candidates, emptyText = "No proposals") {
  const rows = byId("maintenance-rows");
  rows.replaceChildren(...candidates.map(maintenanceCandidateRow));
  if (candidates.length === 0) {
    const row = document.createElement("tr");
    const cell = element("td", "empty", emptyText);
    cell.colSpan = 5;
    row.append(cell);
    rows.append(row);
  }
}

function summaryProposalCard(candidate) {
  const card = element("article", "maintenance-summary-card");
  const heading = element("div", "maintenance-summary-card-heading");
  const selection = element("label", "maintenance-card-select");
  selection.append(maintenanceSelectionCheckbox(candidate), element("span", "", t("Summary proposal")));
  const source = element("div", "maintenance-summary-source");
  source.append(
    element("strong", "", candidate.namespace),
    element("span", "mono muted", candidate.pageId),
  );
  const open = element("button", "compact-button", t("Open page"));
  open.type = "button";
  open.addEventListener("click", () => pageInspector.open(candidate.pageId));
  heading.append(selection, source, open);
  const metadata = element("div", "maintenance-summary-metadata");
  metadata.textContent = `${t("Summary route")} · ${formatSize(candidate.contentChars)} ${t("Page")}`;
  card.append(heading, metadata, element("div", "maintenance-summary-content", candidate.content));
  return card;
}

function renderMaintenanceProposals(candidates) {
  const summaryCandidates = candidates.filter((candidate) => candidate.operation === "summary");
  const tableCandidates = candidates.filter((candidate) => candidate.operation !== "summary");
  const summaryCards = byId("maintenance-summary-cards");
  const table = byId("maintenance-table-wrap");
  summaryCards.hidden = summaryCandidates.length === 0;
  table.hidden = tableCandidates.length === 0;
  if (summaryCandidates.length) summaryCards.replaceChildren(...summaryCandidates.map(summaryProposalCard));
  if (tableCandidates.length) renderMaintenanceCandidateRows(tableCandidates);
}

function updateMaintenanceButton(button, label, loading, activity) {
  const progress = loading && activity?.total > 0
    ? ` ${formatNumber(activity.current)} of ${formatNumber(activity.total)}`
    : "";
  const activeLabel = t({ scan: "Scanning", analyze: "Analyzing", optimize: "Optimizing" }[activity?.kind]) || label;
  button.textContent = loading ? `${activeLabel}${progress}` : label;
  button.classList.toggle("is-loading", loading);
  button.setAttribute("aria-busy", loading ? "true" : "false");
}

function maintenanceStepStatus(phase) {
  const phaseConfig = maintenancePhaseConfig(phase);
  if (maintenanceSessionComplete()) return t("Completed");
  if (!maintenanceSessionActive()) return phase === "pack" ? t("Not started") : t("Waiting");
  const current = maintenancePhaseConfig();
  if (phaseConfig.order < current.order) return t("Completed");
  if (phaseConfig.order > current.order) return t("Waiting");
  if (state.maintenance.busy) return currentLanguage === "zh" ? "处理中" : "Working";
  const scan = maintenanceScanForPhase();
  if (!scan) return t("Waiting");
  const analysis = state.maintenance.analyses[phase];
  if (!analysis) return maintenanceWorkCount() ? t("Ready to analyze") : t("Ready to continue");
  if (phase === "summary" && summaryFailedBatches().length) return t("Analysis incomplete");
  const candidates = maintenanceCandidates();
  const selected = candidates.filter((candidate) => state.maintenance.selected.has(candidate.candidateId)).length;
  return selected ? t("Ready to apply") : t("Ready to continue");
}

function maintenancePhaseDescription() {
  const phase = maintenancePhase();
  const scan = maintenanceScanForPhase();
  const analysis = state.maintenance.analyses[phase];
  if (!scan) return currentLanguage === "zh"
    ? "正在等待扫描完整的可处理库存。"
    : "Waiting to scan the full eligible inventory.";
  if (!analysis && maintenanceWorkCount() === 0) return t("No eligible work");
  if (!analysis) return currentLanguage === "zh"
    ? `扫描已发现 ${formatNumber(maintenanceWorkCount())} 个结构候选。它们不是已建议的变更；点击分析后才会调用模型判断是否应合并、摘要或关联。`
    : `The scan found ${formatNumber(maintenanceWorkCount())} structural candidates. They are not recommendations yet: analysis calls a model to decide whether to pack, summarize, or relate them.`;
  const failedBatches = phase === "summary" ? summaryFailedBatches() : [];
  if (failedBatches.length) return currentLanguage === "zh"
    ? `${formatNumber(failedBatches.length)} 个摘要页面未完成。已完成的提案仍可应用；请重试失败页面，或重新扫描本阶段。`
    : `${formatNumber(failedBatches.length)} summary Page${failedBatches.length === 1 ? " is" : "s are"} incomplete. Completed proposals remain available; retry failed Pages or rescan this stage.`;
  if (maintenanceCandidates().length) return t("Review the proposals below, select the changes to apply, then continue to the next stage.");
  return t("Analysis completed. No changes are recommended for this stage. Continue when you are ready.");
}

function currentMaintenanceAction() {
  if (!maintenanceSessionActive()) return "start";
  const stage = maintenanceWorkflowStage();
  if (stage === "scan") return "scan";
  if (stage === "analyze") return "analyze";
  const selectedCount = maintenanceCandidates()
    .filter((candidate) => state.maintenance.selected.has(candidate.candidateId)).length;
  return selectedCount ? "apply" : "advance";
}

function maintenancePrimaryLabel(action = currentMaintenanceAction()) {
  if (action === "start") return t("Start maintenance");
  if (action === "scan") return t("Scan candidates");
  if (action === "analyze") return t("Analyze suggestions");
  if (action === "retry") return t("Retry failed pages");
  if (action === "apply") {
    const count = maintenanceCandidates().filter((candidate) => state.maintenance.selected.has(candidate.candidateId)).length;
    return `${t("Apply selected")} (${formatNumber(count)})`;
  }
  return maintenancePass() === "pack" ? t("Continue to semantic maintenance") : t("Complete maintenance");
}

function phaseIssue(phase) {
  const analysisIssues = state.maintenance.analyses[phase]?.issues || [];
  // The phase outcome snapshots analysis issues for the report. Prefer the
  // live analysis list so the same worker failure is not shown twice.
  const issues = analysisIssues.length ? analysisIssues : (maintenanceOutcome(phase)?.issues || []);
  if (!issues.length) return null;
  return issues.map((issue) => {
    const message = issue.message || String(issue);
    if (!Number.isInteger(issue.batchIndex)) return message;
    const item = phase === "summary"
      ? (currentLanguage === "zh" ? "页面" : "Page")
      : (currentLanguage === "zh" ? "批次" : "Batch");
    return `${item} ${formatNumber(issue.batchIndex + 1)}: ${message}`;
  }).join("\n");
}

function renderMaintenanceSteps() {
  const activeStage = maintenanceWorkflowStage();
  for (const stage of Object.keys(MAINTENANCE_STAGES)) {
    const step = byId(`maintenance-step-${stage}`);
    const config = maintenanceStageConfig(stage);
    const activeConfig = maintenanceStageConfig();
    step.classList.toggle("active", maintenanceSessionActive() && stage === activeStage);
    step.classList.toggle("completed", maintenanceSessionComplete() || (maintenanceSessionActive() && config.order < activeConfig.order));
    const status = maintenanceSessionComplete() || config.order < activeConfig.order
      ? t("Completed")
      : stage === activeStage && state.maintenance.busy
        ? (currentLanguage === "zh" ? "处理中" : "Working")
        : stage === activeStage
          ? (stage === "review" ? t("Ready to apply") : t("Ready to continue"))
          : t("Waiting");
    byId(`maintenance-step-${stage}-status`).textContent = status;
  }
  const report = byId("maintenance-step-report");
  report.classList.toggle("active", maintenanceSessionComplete());
  report.classList.toggle("completed", maintenanceSessionComplete());
  byId("maintenance-step-report-status").textContent = maintenanceSessionComplete() ? t("Completed") : t("Waiting");
}

function renderMaintenanceWorkflow() {
  const stage = maintenanceWorkflowStage();
  const stageConfig = maintenanceStageConfig();
  const pass = maintenancePass();
  const passConfig = maintenancePassConfig();
  const scans = state.maintenance.scan || {};
  const scanGroups = maintenancePassPhases().reduce((count, phase) => count + (phase === "summary"
    ? 0
    : maintenanceScanForPhase(phase)?.candidateGroupCount || 0), 0);
  const scanPages = pass === "semantic" ? scans.summary?.eligiblePages || 0 : 0;
  const estimatedCalls = maintenancePassPhases()
    .reduce((count, phase) => count + maintenanceEstimatedCalls(phase), 0);
  const analyses = maintenancePassPhases()
    .map((phase) => state.maintenance.analyses[phase])
    .filter(Boolean);
  const modelCalls = analyses.reduce((sum, analysis) => sum + (analysis.workerCalls || 0), 0);
  const issues = analyses.flatMap((analysis) => analysis.issues || []);
  const candidates = maintenanceCandidates();
  const selectedCount = candidates.filter((candidate) => state.maintenance.selected.has(candidate.candidateId)).length;

  renderMaintenanceSteps();
  byId("maintenance-phase-order").textContent = currentLanguage === "zh"
    ? `第 ${passConfig.order} 段 · ${t(passConfig.label)} · 第 ${stageConfig.order} 步，共 3 步`
    : `Pass ${passConfig.order}: ${t(passConfig.label)} · Step ${stageConfig.order} of 3`;
  byId("maintenance-phase-title").textContent = t(passConfig.label);
  byId("maintenance-phase-description").textContent = stage === "scan"
    ? (currentLanguage === "zh"
      ? "读取完整可处理库存，生成结构候选；这一阶段不调用模型，也不写入任何页面。"
      : "Read the full eligible inventory and form structural candidates. This stage makes no model calls or Page writes.")
    : stage === "analyze" && pass === "pack"
      ? (currentLanguage === "zh"
        ? "只分析 Pack 合并建议；不会分析摘要或关联。应用或跳过本段后，才会基于刷新后的库存启动语义维护。"
        : "Analyze only Pack merge proposals. Summary and relation analysis wait until this pass is applied or skipped and the inventory is refreshed.")
    : stage === "analyze"
      ? (currentLanguage === "zh"
        ? "模型评估摘要与关联候选，只提出建议；不会写入页面。"
        : "The model evaluates summary and relation candidates and only proposes changes; it writes nothing.")
      : (currentLanguage === "zh"
        ? "统一审阅所有建议。只会应用你勾选的项目，每项写入前都会重新校验当前版本。"
        : "Review every suggestion together. Only selected items are applied, and each write revalidates the current revision.");

  const metrics = stage === "scan"
    ? [
      metric(t("Scanned candidate groups"), formatNumber(scanGroups)),
      metric(t("Scanned eligible pages"), formatNumber(scanPages)),
      metric(t("Estimated calls"), formatNumber(estimatedCalls)),
    ]
    : [
      metric(t("Scanned candidate groups"), formatNumber(scanGroups)),
      metric(t("Scanned eligible pages"), formatNumber(scanPages)),
      metric(t("Model calls"), formatNumber(modelCalls), modelCalls ? "info" : ""),
      metric(t("Proposals"), formatNumber(stage === "review" ? candidates.length : 0), candidates.length ? "info" : ""),
    ];
  byId("maintenance-scan-metrics").replaceChildren(...metrics);
  byId("maintenance-candidate-status").textContent = stage === "review"
    ? `${formatNumber(candidates.length)} ${t("proposals")} · ${formatNumber(selectedCount)} ${currentLanguage === "zh" ? "已选" : "selected"}`
    : stage === "analyze"
      ? `${formatNumber(modelCalls)} ${t("Model calls")} · ${formatNumber(estimatedCalls)} ${t("estimated model calls")}`
      : `${formatNumber(scanGroups + scanPages)} ${currentLanguage === "zh" ? "扫描项" : "scanned items"}`;

  const issue = issues.map((item) => item.message || String(item)).join("\n");
  const issueNode = byId("maintenance-issue");
  issueNode.hidden = !issue;
  issueNode.textContent = issue ? `${t("Analysis incomplete")}: ${issue}` : "";

  const proposals = byId("maintenance-proposals");
  proposals.hidden = stage !== "review" || candidates.length === 0;
  if (!proposals.hidden) {
    renderMaintenanceProposals(candidates);
    byId("maintenance-selection-status").textContent = currentLanguage === "zh"
      ? `已选 ${formatNumber(selectedCount)} / ${formatNumber(candidates.length)}`
      : `${formatNumber(selectedCount)} of ${formatNumber(candidates.length)} selected`;
  }

  const primary = byId("maintenance-primary");
  const action = currentMaintenanceAction();
  updateMaintenanceButton(primary, maintenancePrimaryLabel(action), state.maintenance.busy, state.maintenance.activity);
  primary.disabled = !maintenanceAvailable() || state.maintenance.busy;
  const rescan = byId("maintenance-rescan");
  rescan.disabled = state.maintenance.busy;
  const skip = byId("maintenance-skip");
  skip.hidden = stage !== "review";
  skip.disabled = state.maintenance.busy;
  const retryFailed = byId("maintenance-retry-failed");
  retryFailed.hidden = true;
  retryFailed.disabled = true;
  const cancel = byId("maintenance-cancel");
  cancel.disabled = state.maintenance.busy;
  const selectAll = byId("maintenance-select-all");
  selectAll.disabled = state.maintenance.busy || candidates.length === 0;
  selectAll.checked = candidates.length > 0 && selectedCount === candidates.length;
  selectAll.indeterminate = selectedCount > 0 && selectedCount < candidates.length;

  if (!state.maintenance.busy) {
    byId("maintenance-status").textContent = `${t(stageConfig.label)} · ${stage === "review" ? t("Ready to apply") : t("Ready to continue")}`;
  }
}

function renderMaintenanceReport() {
  renderMaintenanceSteps();
  const outcomes = state.maintenance.session.outcomes;
  const totalCalls = Object.values(outcomes).reduce((sum, outcome) => sum + outcome.modelCalls, 0);
  const totalSkipped = Object.values(outcomes).reduce((sum, outcome) => sum + outcome.skipped, 0);
  byId("maintenance-report-status").textContent = `${t("Maintenance session completed")} · ${formatTime(state.maintenance.session.completedAt)}`;
  byId("maintenance-report-metrics").replaceChildren(
    metric(t("Pack"), `${formatNumber(outcomes.pack.applied)} ${t("Applied")}`, outcomes.pack.applied ? "positive" : ""),
    metric(t("Summary"), `${formatNumber(outcomes.summary.applied)} ${t("Applied")}`, outcomes.summary.applied ? "positive" : ""),
    metric(t("Relations"), `${formatNumber(outcomes.relation.applied)} ${t("Applied")}`, outcomes.relation.applied ? "positive" : ""),
    metric(t("Model calls"), formatNumber(totalCalls), totalCalls ? "info" : ""),
    metric(t("Skipped"), formatNumber(totalSkipped), totalSkipped ? "warning" : ""),
  );
  byId("maintenance-status").textContent = t("Maintenance session completed");
}

function renderMaintenanceSession() {
  const idle = !maintenanceSessionActive() && !maintenanceSessionComplete();
  byId("maintenance-idle").hidden = !idle;
  byId("maintenance-workflow").hidden = !maintenanceSessionActive();
  byId("maintenance-report").hidden = !maintenanceSessionComplete();
  byId("maintenance-start").disabled = !maintenanceAvailable() || state.maintenance.busy;
  if (idle) {
    byId("maintenance-status").textContent = maintenanceAvailable()
      ? (currentLanguage === "zh" ? "准备开始可审阅的维护会话" : "Ready to start a reviewable maintenance session")
      : t("Unavailable");
    return;
  }
  if (maintenanceSessionActive()) renderMaintenanceWorkflow();
  if (maintenanceSessionComplete()) renderMaintenanceReport();
}

function emptyMaintenanceAnalysis(scan) {
  return {
    analyzedAt: null,
    scanId: scan.scanId,
    batchCount: scan.estimatedModelCalls,
    batchesCompleted: 0,
    candidateGroupCount: scan.candidateGroupCount,
    analyzedGroupCount: 0,
    workerCalls: 0,
    overlapRetries: 0,
    noCandidateGroups: 0,
    deferredGroups: 0,
    candidates: [],
    issues: [],
  };
}

function appendMaintenanceAnalysisBatch(analysis, batch) {
  analysis.analyzedAt = batch.analyzedAt;
  analysis.batchCount = batch.batchCount;
  analysis.batchesCompleted += 1;
  analysis.analyzedGroupCount += batch.analyzedGroupCount;
  analysis.workerCalls += batch.workerCalls;
  analysis.overlapRetries += batch.overlapRetries || 0;
  analysis.noCandidateGroups += batch.noCandidateGroups;
  analysis.deferredGroups += batch.deferredGroups;
  analysis.candidates.push(...batch.candidates);
  if (batch.issue) analysis.issues.push({ ...batch.issue, batchIndex: batch.batchIndex });
}

async function loadMaintenance({ reload = false } = {}) {
  if (!state.maintenance.loaded || reload) renderMaintenanceStatus(await api("/api/maintenance"));
  else renderMaintenanceSession();
  await loadRelationReviews();
}

async function requestMaintenanceScan() {
  return maintenanceMutation("/api/maintenance/scan", {});
}

async function applyMaintenanceCandidates(candidates, onProgress) {
  let applied = 0;
  const appliedCandidateIds = [];
  const skipped = [];
  for (const [index, candidate] of candidates.entries()) {
    onProgress?.({ index, total: candidates.length, applied, skipped });
    try {
      if (candidate.operation === "summary") {
        await maintenanceMutation("/api/maintenance/summaries/apply", {
          candidateId: candidate.candidateId,
          pageId: candidate.pageId,
          revisionId: candidate.revisionId,
          expectedSummaryRevisionId: candidate.expectedSummaryRevisionId,
          content: candidate.content,
        });
      } else if (candidate.operation === "relation") {
        await maintenanceMutation("/api/maintenance/relations/apply", {
          candidateId: candidate.candidateId,
          pages: candidate.pages.map((page) => ({ pageId: page.pageId, revisionId: page.revisionId })),
        });
      } else {
        await maintenanceMutation("/api/maintenance/packs/apply", {
          candidateId: candidate.candidateId,
          pages: candidate.pages.map((page) => ({ pageId: page.pageId, revisionId: page.revisionId })),
        });
      }
      applied += 1;
      appliedCandidateIds.push(candidate.candidateId);
    } catch (error) {
      skipped.push({ candidateId: candidate.candidateId, message: error.message || String(error) });
    }
    onProgress?.({ index: index + 1, total: candidates.length, applied, skipped });
  }
  return { applied, appliedCandidateIds, skipped };
}

function maintenanceBatches(items, batchSize = SUMMARY_REVIEW_BATCH_SIZE) {
  const batches = [];
  for (let index = 0; index < items.length; index += batchSize) {
    batches.push(items.slice(index, index + batchSize));
  }
  return batches;
}

function appendMaintenanceCandidates(operation, candidates) {
  for (const candidate of candidates || []) {
    const proposal = { ...candidate, operation };
    const existing = state.maintenance.pendingCandidates.findIndex((item) => item.candidateId === proposal.candidateId);
    if (existing >= 0) state.maintenance.pendingCandidates.splice(existing, 1, proposal);
    else state.maintenance.pendingCandidates.push(proposal);
    state.maintenance.selected.add(proposal.candidateId);
  }
}

function emptySummaryAnalysis(scan) {
  const batches = maintenanceBatches(scan.pages);
  return {
    analyzedAt: null,
    scanId: scan.scanId,
    batchCount: batches.length,
    batchesCompleted: 0,
    workerCalls: 0,
    noCandidatePages: 0,
    deferredPages: 0,
    issues: [],
    batches: batches.map((pages, batchIndex) => ({
      batchIndex,
      pageCount: pages.length,
      status: "pending",
      attempts: 0,
      workerCalls: 0,
      noCandidatePages: 0,
      deferredPages: 0,
      issues: [],
    })),
  };
}

function summaryFailedBatches() {
  return (state.maintenance.analyses.summary?.batches || [])
    .filter((batch) => batch.status === "failed");
}

function refreshSummaryAnalysisTotals(analysis) {
  analysis.batchesCompleted = analysis.batches.filter((batch) => batch.status === "completed").length;
  analysis.workerCalls = analysis.batches.reduce((total, batch) => total + batch.workerCalls, 0);
  analysis.noCandidatePages = analysis.batches.reduce((total, batch) => total + batch.noCandidatePages, 0);
  analysis.deferredPages = analysis.batches.reduce((total, batch) => total + batch.deferredPages, 0);
  analysis.issues = analysis.batches.flatMap((batch) => batch.issues);
}

function emptyRelationAnalysis(scan) {
  return {
    analyzedAt: null,
    batchCount: scan.estimatedModelCalls,
    batchesCompleted: 0,
    workerCalls: 0,
    noCandidateGroups: 0,
    deferredGroups: 0,
    issues: [],
  };
}

function resetCurrentMaintenanceWork() {
  state.maintenance.scan = null;
  state.maintenance.analyses = { pack: null, summary: null, relation: null };
  state.maintenance.pendingCandidates = [];
  state.maintenance.selected.clear();
}

async function scanMaintenance() {
  if (state.maintenance.busy || !maintenanceAvailable()) return;
  resetCurrentMaintenanceWork();
  state.maintenance.busy = true;
  state.maintenance.activity = { kind: "scan", current: 0, total: 1 };
  byId("maintenance-status").textContent = `${t("Scanning")} ${t(maintenancePassConfig().label)}`;
  renderMaintenanceSession();
  try {
    const scan = await requestMaintenanceScan();
    state.maintenance.activity.current = 1;
    state.maintenance.scan = scan;
    for (const phase of maintenancePassPhases()) {
      const outcome = maintenanceOutcome(phase);
      outcome.scannedAt = scan.capturedAt;
      outcome.workItems = maintenanceWorkCount(phase);
    }
    state.maintenance.phase = maintenancePassPhases()[0];
    state.maintenance.workflowStage = "analyze";
    byId("maintenance-status").textContent = `${t("Scan complete")} · ${formatTime(scan.capturedAt)}`;
  } catch (error) {
    byId("maintenance-status").textContent = `${t("Scan failed")}: ${error.message || String(error)}`;
    showError(error);
  } finally {
    state.maintenance.busy = false;
    state.maintenance.activity = null;
    renderMaintenanceSession();
  }
}

async function analyzePackingPhase(scan) {
  const analysis = emptyMaintenanceAnalysis(scan);
  state.maintenance.analyses.pack = analysis;
  // Mechanical Pack merges are valid proposals with zero model calls. Keep
  // them in the same review queue as model-selected Pack candidates so a zero
  // worker count can never look like this pass was skipped.
  for (let batchIndex = 0; batchIndex < analysis.batchCount; batchIndex += 1) {
    state.maintenance.activity.current = batchIndex + 1;
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Pack")} ${formatNumber(batchIndex + 1)} / ${formatNumber(analysis.batchCount)}`;
    renderMaintenanceSession();
    const batch = await maintenanceMutation("/api/maintenance/analyze", {
      scanId: scan.scanId,
      batchIndex,
    });
    appendMaintenanceAnalysisBatch(analysis, batch);
    appendMaintenanceCandidates("pack", batch.candidates);
    renderMaintenanceSession();
  }
  state.maintenance.analyses.pack = analysis;
}

async function analyzeSummaryPhase(scan, { retryFailed = false } = {}) {
  let analysis = state.maintenance.analyses.summary;
  if (!retryFailed || !analysis || analysis.scanId !== scan.scanId) {
    analysis = emptySummaryAnalysis(scan);
    state.maintenance.analyses.summary = analysis;
  }
  const batches = maintenanceBatches(scan.pages);
  const batchIndexes = retryFailed
    ? analysis.batches.filter((batch) => batch.status === "failed").map((batch) => batch.batchIndex)
    : analysis.batches.filter((batch) => batch.status !== "completed").map((batch) => batch.batchIndex);
  for (const [progressIndex, index] of batchIndexes.entries()) {
    const batch = batches[index];
    const batchState = analysis.batches[index];
    if (!batch || !batchState) continue;
    batchState.status = "running";
    batchState.attempts += 1;
    batchState.issues = [];
    batchState.deferredPages = 0;
    batchState.noCandidatePages = 0;
    batchState.workerCalls = 0;
    state.maintenance.activity.current = progressIndex + 1;
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Summary")} ${formatNumber(index + 1)} / ${formatNumber(batches.length)}`;
    renderMaintenanceSession();
    try {
      const result = await maintenanceMutation("/api/maintenance/summaries/analyze-batch", {
        scanId: scan.scanId,
        pages: batch.map((item) => ({ pageId: item.pageId, revisionId: item.revisionId })),
      });
      appendMaintenanceCandidates("summary", result.candidates);
      analysis.analyzedAt = result.analyzedAt;
      batchState.workerCalls = result.workerCalls || 0;
      batchState.noCandidatePages = result.noCandidatePages || 0;
      batchState.deferredPages = result.deferredPages || 0;
      batchState.issues = (result.issues || []).map((issue) => ({
        ...issue,
        batchIndex: index,
        message: issue.message || String(issue),
      }));
      batchState.status = batchState.issues.length ? "failed" : "completed";
    } catch (error) {
      batchState.deferredPages = batch.length;
      batchState.issues = [{ batchIndex: index, message: error.message || String(error) }];
      batchState.status = "failed";
    }
    refreshSummaryAnalysisTotals(analysis);
    renderMaintenanceSession();
  }
}

async function analyzeRelationPhase(scan) {
  const analysis = emptyRelationAnalysis(scan);
  state.maintenance.analyses.relation = analysis;
  for (const [index, group] of scan.groups.entries()) {
    state.maintenance.activity.current = index + 1;
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Relations")} ${formatNumber(index + 1)} / ${formatNumber(scan.groups.length)}`;
    renderMaintenanceSession();
    try {
      const result = await maintenanceMutation("/api/maintenance/relations/analyze", {
        scanId: scan.scanId,
        groupId: group.groupId,
      });
      analysis.analyzedAt = result.analyzedAt;
      analysis.workerCalls += 1;
      if (result.candidate) appendMaintenanceCandidates("relation", [result.candidate]);
      else if (result.decision === "defer") analysis.deferredGroups += 1;
      else analysis.noCandidateGroups += 1;
    } catch (error) {
      analysis.workerCalls += 1;
      analysis.deferredGroups += 1;
      analysis.issues.push({ batchIndex: index, message: error.message || String(error) });
    }
    analysis.batchesCompleted += 1;
    renderMaintenanceSession();
  }
}

async function analyzeMaintenance() {
  if (state.maintenance.busy || !state.maintenance.scan || maintenanceWorkflowStage() !== "analyze") return;
  state.maintenance.busy = true;
  const total = maintenancePassPhases()
    .reduce((count, phase) => count + maintenanceEstimatedCalls(phase), 0);
  state.maintenance.activity = { kind: "analyze", current: 0, total };
  byId("maintenance-status").textContent = t("Preparing");
  renderMaintenanceSession();
  try {
    for (const phase of maintenancePassPhases()) {
      state.maintenance.phase = phase;
      const scan = maintenanceScanForPhase(phase);
      if (!scan || maintenanceWorkCount(phase) === 0) continue;
      if (phase === "pack") await analyzePackingPhase(scan);
      else if (phase === "summary") await analyzeSummaryPhase(scan);
      else await analyzeRelationPhase(scan);
      const analysis = state.maintenance.analyses[phase];
      const outcome = maintenanceOutcome(phase);
      outcome.analyzedAt = analysis?.analyzedAt || new Date().toISOString();
      outcome.modelCalls += analysis?.workerCalls || 0;
      outcome.proposals = phase === "pack"
        ? (analysis?.candidates.length || 0)
        : state.maintenance.pendingCandidates.filter((candidate) => candidate.operation === phase).length;
      outcome.issues = [...(analysis?.issues || [])];
    }
    state.maintenance.workflowStage = "review";
    state.maintenance.phase = maintenancePassPhases()[0];
    for (const candidate of maintenanceCandidates()) state.maintenance.selected.add(candidate.candidateId);
  } catch (error) {
    const analysis = state.maintenance.analyses[maintenancePhase()];
    if (analysis) analysis.issues.push({ message: error.message || String(error) });
    byId("maintenance-status").textContent = `${t("Analysis failed")}: ${error.message || String(error)}`;
    showError(error);
  } finally {
    state.maintenance.busy = false;
    state.maintenance.activity = null;
    renderMaintenanceSession();
  }
}

async function retryFailedSummaryBatches() {
  const scan = maintenanceScanForPhase();
  const failed = summaryFailedBatches();
  if (state.maintenance.busy || maintenancePhase() !== "summary" || !scan || failed.length === 0) return;
  state.maintenance.busy = true;
  state.maintenance.activity = { kind: "analyze", current: 0, total: failed.length };
  byId("maintenance-status").textContent = `${t("Preparing")} ${t("Summary")}`;
  renderMaintenanceSession();
  try {
    const previousCalls = state.maintenance.analyses.summary?.workerCalls || 0;
    await analyzeSummaryPhase(scan, { retryFailed: true });
    for (const candidate of maintenanceCandidates()) state.maintenance.selected.add(candidate.candidateId);
    const analysis = state.maintenance.analyses.summary;
    const outcome = maintenanceOutcome();
    outcome.analyzedAt = analysis?.analyzedAt || outcome.analyzedAt || new Date().toISOString();
    outcome.modelCalls += Math.max(0, (analysis?.workerCalls || 0) - previousCalls);
    outcome.proposals = maintenanceCandidates().length;
    outcome.issues = [...(analysis?.issues || [])];
  } catch (error) {
    byId("maintenance-status").textContent = `${t("Analysis failed")}: ${error.message || String(error)}`;
    showError(error);
  } finally {
    state.maintenance.busy = false;
    state.maintenance.activity = null;
    renderMaintenanceSession();
  }
}

async function optimizeMaintenanceSelection() {
  if (state.maintenance.busy) return;
  const candidates = maintenanceCandidates()
    .filter((candidate) => state.maintenance.selected.has(candidate.candidateId));
  if (candidates.length === 0) return;
  const candidateCount = formatNumber(candidates.length);
  const singular = candidates.length === 1;
  const phaseLabel = t(maintenancePhaseConfig().label);
  const confirmed = await confirmAction({
    title: currentLanguage === "zh"
      ? `优化选中的 ${candidateCount} 个${phaseLabel}提案？`
      : `Optimize ${candidateCount} selected ${phaseLabel} proposal${singular ? "" : "s"}?`,
    description: currentLanguage === "zh"
      ? "只会提交当前阶段中已选择的提案。每项都会在写入前校验当前版本；完成后由你决定何时继续到下一阶段。"
      : "Only selected proposals in the current phase will be applied. Each write checks the current revision first; you choose when to continue to the next stage.",
    confirmLabel: t("Apply selected"),
  });
  if (!confirmed) return;

  state.maintenance.busy = true;
  state.maintenance.activity = { kind: "optimize", current: 0, total: candidates.length };
  byId("maintenance-status").textContent = `Optimizing 0 of ${formatNumber(candidates.length)}`;
  renderMaintenanceSession();
  let result = { applied: 0, appliedCandidateIds: [], skipped: [] };
  try {
    result = await applyMaintenanceCandidates(candidates, ({ index, total, applied, skipped }) => {
      state.maintenance.activity.current = index;
      byId("maintenance-status").textContent = `Optimizing ${formatNumber(index)} of ${formatNumber(total)} · ${formatNumber(applied)} applied · ${formatNumber(skipped.length)} skipped`;
      renderMaintenanceSession();
    });
    state.maintenance.selected.clear();
    const consumed = new Set([...result.appliedCandidateIds, ...result.skipped.map((item) => item.candidateId)]);
    if (maintenanceWorkflowStage() === "review") {
      const analysis = state.maintenance.analyses.pack;
      if (analysis) analysis.candidates = analysis.candidates.filter((candidate) => !consumed.has(candidate.candidateId));
      state.maintenance.pendingCandidates = state.maintenance.pendingCandidates
        .filter((candidate) => !consumed.has(candidate.candidateId));
    }
    const byCandidateId = new Map(candidates.map((candidate) => [candidate.candidateId, candidate]));
    for (const candidateId of result.appliedCandidateIds) {
      const candidate = byCandidateId.get(candidateId);
      if (candidate) maintenanceOutcome(candidate.operation).applied += 1;
    }
    for (const skipped of result.skipped) {
      const candidate = byCandidateId.get(skipped.candidateId);
      if (candidate) {
        const outcome = maintenanceOutcome(candidate.operation);
        outcome.skipped += 1;
        outcome.issues.push(skipped);
      }
    }
    await loadOverview();
    const outcomeText = `${formatNumber(result.applied)} applied · ${formatNumber(result.skipped.length)} skipped`;
    byId("maintenance-status").textContent = `${t("Optimization completed")} · ${outcomeText}`;
    if (result.skipped.length > 0) {
      showError(new Error(`${result.skipped.length} maintenance proposal${result.skipped.length === 1 ? " was" : "s were"} skipped. First issue: ${result.skipped[0].message}`));
    }
  } catch (error) {
    byId("maintenance-status").textContent = `Optimization failed after ${formatNumber(result.applied)} applied · ${formatNumber(result.skipped.length)} skipped`;
    showError(error);
  } finally {
    state.maintenance.busy = false;
    state.maintenance.activity = null;
    renderMaintenanceSession();
  }
}

function toggleMaintenanceSelection() {
  const candidates = maintenanceCandidates();
  const allSelected = candidates.length > 0
    && candidates.every((candidate) => state.maintenance.selected.has(candidate.candidateId));
  state.maintenance.selected.clear();
  if (!allSelected) {
    for (const candidate of candidates) state.maintenance.selected.add(candidate.candidateId);
  }
  renderMaintenanceSession();
}

async function startMaintenanceSession() {
  if (state.maintenance.busy || !maintenanceAvailable()) return;
  resetMaintenanceSession();
  state.maintenance.session.state = "active";
  state.maintenance.session.startedAt = new Date().toISOString();
  byId("maintenance-status").textContent = t("Maintenance session started");
  renderMaintenanceSession();
  await scanMaintenance();
}

async function advanceMaintenancePhase() {
  if (!maintenanceSessionActive() || state.maintenance.busy) return;
  const candidates = maintenanceCandidates();
  if (candidates.length > 0) {
    const confirmed = await confirmAction({
      title: currentLanguage === "zh" ? "跳过未应用的提案？" : "Continue without applying remaining proposals?",
      description: currentLanguage === "zh"
        ? `本段还有 ${formatNumber(candidates.length)} 个未应用提案。它们不会写入；下一维护段会重新扫描实际库存。`
        : `${formatNumber(candidates.length)} proposals remain unapplied. They will not be written; the next maintenance pass will rescan the actual inventory.`,
      confirmLabel: t("Continue"),
    });
    if (!confirmed) return;
  }
  await completeMaintenancePhase({ skippedProposals: candidates.length });
}

async function skipMaintenancePhase() {
  if (!maintenanceSessionActive() || state.maintenance.busy) return;
  const pass = maintenancePassConfig();
  const candidates = maintenanceCandidates();
  const confirmed = await confirmAction({
    title: currentLanguage === "zh" ? `跳过${t(pass.label)}？` : `Skip ${t(pass.label)}?`,
    description: currentLanguage === "zh"
      ? "不会再调用模型或写入页面。本段未应用提案将被丢弃；下一维护段会重新扫描实际库存。"
      : "No additional model call or Page write will occur. Unapplied proposals in this pass are discarded; the next pass rescans the actual inventory.",
    confirmLabel: t("Skip this stage"),
  });
  if (!confirmed) return;
  await completeMaintenancePhase({ skippedProposals: candidates.length });
}

async function completeMaintenancePhase({ skippedProposals = 0 } = {}) {
  const candidates = maintenanceCandidates();
  const skippedByOperation = new Map();
  for (const candidate of candidates) {
    skippedByOperation.set(candidate.operation, (skippedByOperation.get(candidate.operation) || 0) + 1);
  }
  for (const phase of maintenancePassPhases()) {
    const outcome = maintenanceOutcome(phase);
    outcome.skipped += skippedByOperation.get(phase) || 0;
    outcome.completed = true;
  }
  if (maintenancePass() === "pack") {
    state.maintenance.pass = "semantic";
    state.maintenance.phase = "summary";
    state.maintenance.workflowStage = "scan";
    resetCurrentMaintenanceWork();
    renderMaintenanceSession();
    await scanMaintenance();
    return;
  }
  state.maintenance.session.state = "complete";
  state.maintenance.session.completedAt = new Date().toISOString();
  renderMaintenanceSession();
}

async function rescanMaintenancePhase() {
  if (!maintenanceSessionActive() || state.maintenance.busy) return;
  const hasAnalysis = Boolean(state.maintenance.analyses[maintenancePhase()]);
  if (hasAnalysis) {
    const confirmed = await confirmAction({
      title: currentLanguage === "zh" ? "重新扫描本阶段？" : "Rescan this stage?",
      description: currentLanguage === "zh"
        ? "重新扫描会丢弃本阶段尚未应用的提案，并从当前 Store 重建工作集。"
        : "Rescanning discards unapplied proposals for this stage and rebuilds the work set from the current Store.",
      confirmLabel: t("Rescan this stage"),
    });
    if (!confirmed) return;
  }
  await scanMaintenance();
}

async function cancelMaintenanceSession() {
  if (!maintenanceSessionActive() || state.maintenance.busy) return;
  const confirmed = await confirmAction({
    title: t("End this maintenance session?"),
    description: t("No additional Page writes will occur. Unapplied proposals remain unapplied."),
    confirmLabel: t("End session"),
  });
  if (!confirmed) return;
  resetMaintenanceSession();
  renderMaintenanceSession();
}

async function runMaintenancePrimaryAction() {
  const action = currentMaintenanceAction();
  if (action === "start") return startMaintenanceSession();
  if (action === "scan") return scanMaintenance();
  if (action === "analyze") return analyzeMaintenance();
  if (action === "retry") return retryFailedSummaryBatches();
  if (action === "apply") return optimizeMaintenanceSelection();
  return advanceMaintenancePhase();
}

async function activateView(name, { reload = false } = {}) {
  state.activeView = name;
  document.querySelectorAll(".tab").forEach((tab) => tab.classList.toggle("active", tab.dataset.view === name));
  document.querySelectorAll(".view").forEach((view) => view.classList.toggle("active", view.id === `view-${name}`));
  if (name === "pages" && (reload || !state.pages.loaded)) await loadPages();
  if (name === "query") await queryView.load({ reload });
  if (name === "maintenance") await loadMaintenance({ reload });
  if (name === "health") {
    await healthView.load({ reload });
  }
  if (name === "access" && (reload || !state.access.loaded)) await loadAccess();
}

async function openScope(namespace) {
  state.pages.scope = namespace;
  byId("query").value = "";
  renderPageScopeOptions();
  resetPages();
  await activateView("pages", { reload: true });
}

async function refresh() {
  try {
    await loadRuntimeControl();
    await loadOverview();
    if (state.activeView === "pages") {
      resetPages();
      await loadPages();
    }
    if (state.activeView === "query") await queryView.load({ reload: true });
    if (state.activeView === "maintenance") await loadMaintenance({ reload: true });
    if (state.activeView === "health") {
      await healthView.load({ reload: true });
      await retentionView.refreshIfOpen();
    }
    if (state.activeView === "access") await loadAccess();
  } catch (error) { showError(error); }
}

function rerenderForLocale() {
  if (state.overview) renderOverview(state.overview);
  const currentPage = state.pages.pageCache.get(state.pages.page);
  if (currentPage) renderPages(currentPage, state.pages.page);
  queryView.rerender();
  if (state.maintenance.loaded) {
    renderAutomationStatus();
    renderMaintenanceSession();
  }
  healthView.rerender();
  retentionView.rerender();
}

async function loadRuntimeControl() {
  const control = byId("runtime-restart");
  const status = await api("/api/runtime");
  control.hidden = !status.lifecycle.managed;
  control.disabled = !status.lifecycle.ownsProcess;
  control.title = status.lifecycle.ownsProcess
    ? t("Restart the PCP Runtime managed by this Console")
    : t("This Console does not own the current Runtime");
  control.setAttribute("aria-label", control.title);
}

async function restartRuntime() {
  const control = byId("runtime-restart");
  control.disabled = true;
  try {
    await api("/api/runtime/restart", {
      method: "POST",
      headers: { "X-PCP-Console": "1" },
    });
    await refresh();
  } catch (error) {
    showError(error);
    await loadRuntimeControl().catch(() => {});
  }
}

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => activateView(tab.dataset.view).catch(showError));
});
byId("refresh").addEventListener("click", refresh);
byId("runtime-restart").addEventListener("click", restartRuntime);
byId("enrollment-open").addEventListener("click", () => {
  byId("enrollment-dialog").showModal();
  loadEnrollment({ autoOpen: false });
});
byId("enrollment-close").addEventListener("click", () => byId("enrollment-dialog").close());
byId("page-search").addEventListener("submit", (event) => {
  event.preventDefault();
  resetPages();
  loadPages().catch(showError);
});
byId("page-sort-direction").addEventListener("click", () => {
  setPageSortDirection(byId("page-sort-direction").dataset.direction !== "descending");
  resetPages();
  loadPages().catch(showError);
});
byId("page-filter-toggle").addEventListener("click", () => togglePageMenu("page-filter-toggle", "page-filter-menu"));
byId("page-sort-options").addEventListener("click", () => togglePageMenu("page-sort-options", "page-sort-menu"));
byId("page-filter-options").addEventListener("click", (event) => {
  const choice = event.target.closest("button[data-page-scope]");
  if (!choice) return;
  state.pages.scope = choice.dataset.pageScope;
  renderPageScopeOptions();
  closePageMenus();
  resetPages();
  loadPages().catch(showError);
});
byId("page-sort-options-list").addEventListener("click", (event) => {
  const choice = event.target.closest("button[data-page-sort-key]");
  if (!choice) return;
  state.pages.sortKey = choice.dataset.pageSortKey;
  renderPageSortOptions();
  closePageMenus();
  resetPages();
  loadPages().catch(showError);
});
document.addEventListener("click", (event) => {
  if (!event.target.closest(".page-control-menu")) closePageMenus();
});
byId("pages-previous").addEventListener("click", () => loadPages({ page: state.pages.page - 1 }).catch(showError));
byId("pages-next").addEventListener("click", () => loadPages({ page: state.pages.page + 1 }).catch(showError));
byId("maintenance-start").addEventListener("click", () => startMaintenanceSession().catch(showError));
byId("maintenance-primary").addEventListener("click", () => runMaintenancePrimaryAction().catch(showError));
byId("maintenance-skip").addEventListener("click", () => skipMaintenancePhase().catch(showError));
byId("maintenance-retry-failed").addEventListener("click", () => retryFailedSummaryBatches().catch(showError));
byId("maintenance-rescan").addEventListener("click", () => rescanMaintenancePhase().catch(showError));
byId("maintenance-cancel").addEventListener("click", () => cancelMaintenanceSession().catch(showError));
byId("maintenance-start-new").addEventListener("click", () => startMaintenanceSession().catch(showError));
byId("maintenance-settings-form").addEventListener("submit", (event) => saveMaintenanceSettings(event).catch(showError));
byId("maintenance-select-all").addEventListener("change", toggleMaintenanceSelection);
byId("access-more").addEventListener("click", () => loadAccess({ append: true }).catch(showError));
byId("health-window").addEventListener("change", () => healthView.load({ reload: true }).catch(showError));
refresh();
loadEnrollment();
window.setInterval(() => loadEnrollment(), 3000);
window.setInterval(() => {
  if (state.activeView === "maintenance" && !state.maintenance.busy) {
    loadMaintenance({ reload: true }).catch(showError);
  }
}, 15_000);
