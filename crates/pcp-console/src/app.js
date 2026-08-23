import { createPageInspector } from "/page-inspector.js?v=20260823.1";
import { describePagePayload, pagePayloadPreviewText } from "/page-content.js?v=20260822.1";
import { createHealthView } from "/health-view.js?v=20260816.3";
import { createRetentionView } from "/retention-view.js?v=20260818.1";
import { createQueryView } from "/query-view.js?v=20260823.2";
import {
  RELATION_DECISION,
  partitionMaintenanceDecisions,
  stageRelationDecision,
} from "/maintenance-relation-decisions.js?v=20260823.1";
import {
  batchProgress,
  beginBatch,
  completeBatch,
  failBatch,
  runnableBatchIndexes,
} from "/progressive-operation.js?v=20260823.2";
import {
  convergencePhase,
  convergenceSettled,
  mergeConvergenceReport,
} from "/maintenance-convergence.js?v=20260824.1";
import {
  REVIEW_DECISION,
  partitionReviewSession,
  restoreReviewDecisions,
  reviewDecisionCounts,
  serializeReviewDecisions,
  stageReviewDecision,
  undoReviewDecision,
} from "/maintenance-review-session.js?v=20260824.1";

const DEFAULT_PAGE_LIMIT = 20;
const PAGE_LIMIT_OPTIONS = new Set([10, 20, 30]);
const ACCESS_LIMIT = 50;
// Local summary workers receive one Page at a time so an incomplete response
// affects only that Page and can be retried independently.
const SUMMARY_REVIEW_BATCH_SIZE = 1;
const THEME_STORAGE_KEY = "pcp-console.theme";
const LANGUAGE_STORAGE_KEY = "pcp-console.language";
const PAGE_LIMIT_STORAGE_KEY = "pcp-console.pages-per-page";
const REVIEW_SESSION_STORAGE_KEY = "pcp-console.maintenance-review-session.v1";
let maintenanceStatusPoll = null;
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
  "Runtime updates this status; relation proposals still require approval.": "状态由 Runtime 更新；关联提案仍需批准。",
  "Background maintenance and Run now use the same controller and persistent review inbox.": "后台维护与立即运行共用同一控制器和持久审阅箱。",
  "Maintain Pack boundaries, semantic structure, then topic Pages in order. Suggested links are never retrievable before approval.": "按顺序维护 Pack 边界、语义结构与主题页；建议关联仍需批准后才会参与检索。",
  "PCP advances maintenance one bounded job at a time. Safe structural work is applied automatically; uncertain semantic and governance decisions collect here for review.": "PCP 每次推进一个有界维护任务；安全的结构工作自动应用，不确定的语义与治理决策统一进入审阅箱。",
  "Not started": "尚未开始",
  "Waiting": "等待写入",
  "Running": "正在执行",
  "Failed": "失败",
  "Stale": "状态过期",
  "Disabled": "已禁用",
  "Maintenance inventory": "维护库存页",
  "Includes retained superseded Pages for maintenance review.": "包含仍保留供维护核查的已替代页面。",
  "Dirty regions": "待整理范围",
  "Ready regions": "已就绪范围",
  "Pending relation review": "待审关联",
  "Pending review": "待审决策",
  "Maintenance review inbox": "维护审阅箱",
  "Review decisions stay reversible in this session. Nothing changes in the Store until you finish and apply the review.": "审阅决定在本次会话中始终可以撤销；只有完成并应用审阅后，才会写入 Store。",
  "Review progress": "审阅进度",
  "Decided": "已决定",
  "Remaining": "待处理",
  "Pending commit": "等待提交",
  "Finish review and apply": "完成审阅并应用",
  "Finish this review session?": "完成本次审阅？",
  "The staged decisions below will now be applied. This is the point where they become persistent.": "下方暂存决定现在将被逐项应用；从这一步开始，它们会成为持久记录。",
  "Undo": "撤销",
  "Undo all": "全部撤销",
  "Accepted for this review": "本次接受",
  "Rejected for this review": "本次拒绝",
  "Deferred for now": "暂时延后",
  "Will be suppressed": "将不再建议",
  "No decision has been written yet.": "当前尚未写入任何决定。",
  "Review decisions were applied": "审阅决定已应用",
  "Some review decisions could not be applied and remain reversible.": "部分审阅决定未能应用，仍保留为可撤销状态。",
  "Converge memory maintenance": "收敛记忆维护",
  "Run now progressively processes one bounded job per response. You can review accumulated decisions immediately while the remaining work continues.": "立即运行会逐次处理任务，每次响应只推进一个有界工作单元；已有决策可立刻审阅，其余工作继续推进。",
  "Run now": "立即运行",
  "Awaiting run": "等待运行",
  "Run in progress": "正在运行",
  "Awaiting review": "等待审阅",
  "Converged": "已收敛",
  "Needs attention": "需要处理",
  "Retry maintenance": "重试维护",
  "Maintenance run paused": "维护运行已暂停",
  "Relation analysis paused": "关联分析已暂停",
  "The model returned a Page pair outside the current candidate window. No relation was applied for this work unit.": "模型返回的页面组合不属于当前候选窗口；这个工作单元没有应用任何关联。",
  "The current maintenance run stopped before convergence. Completed work and accumulated reviews are preserved.": "本次维护在收敛前停止；已完成的工作和已经积累的审阅项均已保留。",
  "Manual staged review": "手动分阶段审阅",
  "Advanced manual archive review": "高级手动归档审阅",
  "Optional": "可选",
  "Model escalation": "模型升级",
  "Baseline model": "基础模型",
  "Escalated model": "升级模型",
  "Skip for now": "暂时跳过",
  "Review inbox is clear": "审阅箱已清空",
  "Maintenance converged": "维护已收敛",
  "Uncertain relation": "不确定关联",
  "Manual approval required": "需要人工批准",
  "Relation comparison": "关联内容对照",
  "Why this relation is worth review": "为什么值得建立这条关联",
  "No relation rationale was supplied.": "未提供关联判断依据。",
  "A relation comparison requires exactly two Pages.": "关联对照必须恰好包含两个页面。",
  "Left Page": "左侧页面",
  "Right Page": "右侧页面",
  "Compare Pages": "对照页面",
  "Review source Pages": "审阅来源页面",
  "Topic extraction review": "凝练新页审阅",
  "Topic Page proposal": "拟凝练主题页",
  "Why this Topic Page is worth creating": "为什么值得凝练为新页",
  "No Topic rationale was supplied.": "未提供凝练判断依据。",
  "A Topic extraction review requires at least two Pages.": "凝练新页审阅至少需要两个页面。",
  "Source Pages": "来源页面",
  "Expand all entries": "展开全部内容",
  "Collapse all entries": "收起全部内容",
  "Expand full Page": "展开完整页面",
  "Loading full Page…": "正在加载完整页面…",
  "Open in inspector": "在检查器中打开",
  "Write trigger": "写入触发条件",
  "Last completed": "最近完成",
  "Next automatic check": "下次自动检查",
  "Idle backoff": "空闲退避",
  "Write activity wakes Runtime early.": "新的写入会提前唤醒 Runtime。",
  "Consecutive failures": "连续失败",
  "Awaiting the first Runtime heartbeat.": "等待 Runtime 首次心跳。",
  "Approve": "批准",
  "Accept": "接受",
  "Accepted": "已接受",
  "Apply decisions": "应用决策",
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
  "Query activity": "查询活动",
  "Semantic calls": "语义调用",
  "Intent calls": "意图调用",
  "Total Router tokens": "Router 总 token",
  "Query Router tokens": "查询 Router Token",
  "Model usage": "模型用量",
  "Model calls": "模型调用",
  "Token reporting": "Token 报告覆盖",
  "Reported tokens": "已报告 Token",
  "By workflow": "按工作流",
  "Intent matching": "意图匹配",
  "Manual maintenance": "手动维护",
  "Automatic maintenance": "自动维护",
  "No model usage was observed in this window": "此时间范围内未记录模型用量",
  "Recent query calls": "最近查询调用",
  "privacy-preserving": "不保存查询文本或页面内容",
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
  "Content governance": "内容治理",
  "Content governance state": "内容治理状态",
  "Archive removes a Page from default retrieval and graph expansion without deleting it. Direct reads remain available for review and restoration.": "归档会将页面从默认检索和图扩展中移除，但不会删除内容；仍可按 ID 审阅与恢复。",
  "Archive is a manual, reviewable lifecycle action. It removes a Page from default retrieval and graph expansion without deleting it.": "归档是手动、可审阅的生命周期操作：它会从默认检索和图扩展中移除页面，但不会删除内容。",
  "Review archive candidates": "审阅归档候选",
  "Candidates are conservatively identified from current structural evidence. PCP does not store per-Page access metrics, so no usage value is invented.": "候选项只根据当前结构证据保守识别。PCP 不保存逐页面访问数据，因此不会虚构使用价值判断。",
  "Archive review stages": "归档审阅阶段",
  "Archive selected": "归档所选",
  "Archive proposals": "归档提案",
  "Archive review complete": "归档审阅完成",
  "Select all archive candidates": "选择全部归档候选",
  "Select archive candidate": "选择归档候选",
  "Available after maintenance": "将在当前维护会话结束后可用",
  "Continue analysis": "继续分析",
  "No archive proposals": "没有归档提案",
  "Archived": "已归档",
  "Retained": "保留",
  "Deferred": "延后",
  "Analyzed": "已分析",
  "Working": "处理中",
  "Scanning candidates": "正在扫描候选",
  "Archive selected Pages?": "归档所选页面？",
  "Archiving removes the selected Pages from default retrieval, graph expansion, and ordinary maintenance. It does not delete them; they remain available for direct review and restoration.": "归档会将所选页面从默认检索、图扩展和常规维护中移除，但不会删除；仍可直接审阅和恢复。",
  "Human-approved archive review": "人工批准的归档审阅",
  "Archiving": "正在归档",
  "Archive is reversible and auditable. It does not delete content, summaries, provenance, or asserted relations.": "归档可恢复且可审计；不会删除内容、摘要、溯源或已确认关联。",
  "Active Pages": "活跃页面",
  "Archived Pages": "已归档页面",
  "Archive": "归档",
  "Restore": "恢复",
  "Archive reason": "归档理由",
  "Restore reason": "恢复理由",
  "An archive reason is required": "请填写归档理由",
  "A restore reason is required": "请填写恢复理由",
  "Archive this Page?": "归档此页面？",
  "Restore this Page?": "恢复此页面？",
  "Archiving excludes this Page from default retrieval, graph expansion, and maintenance without deleting it.": "归档会将此页面排除在默认检索、图扩展和维护之外，但不会删除它。",
  "Restoring makes this Page eligible for default retrieval and graph expansion again.": "恢复后，此页面会重新参与默认检索和图扩展。",
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
  "Eligible Pages": "可处理页面",
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
  "Rejected for this review": "本次拒绝",
  "Skip for now": "本次跳过",
  "Skipped for now": "本次跳过",
  "Relation proposal": "关联提案",
  "Relation decision": "关联决策",
  "View relation Pages": "查看关联页面",
  "Analysis log": "分析日志",
  "Undo no-suggest": "撤销不再建议",
  "Will not be suggested when applied": "将在应用时不再建议",
  "Reject": "拒绝",
  "Do not suggest this relation again": "不再建议此关联",
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
  "retrievable pages": "可检索页",
  "Query": "查询",
  "Query failed": "查询失败",
  "No query yet": "尚未查询",
  "Search all authorized context, then review the deterministic context pack.": "检索所有已授权上下文，再审阅确定性组装的 Context Pack。",
  "Query intent": "查询意图",
  "Retrieval method": "检索方式",
  "Semantic search": "语义搜索",
  "Match intent": "意图匹配",
  "Retrieval effort": "检索投入",
  "Low": "低",
  "Medium": "中",
  "High": "高",
  "Describe what you need to recall": "描述你希望找回的内容",
  "All authorized scopes": "所有已授权范围",
  "Top results": "结果数量",
  "Build context pack": "组装 Context Pack",
  "Try again": "重试",
  "Matching intent…": "正在匹配意图…",
  "Searching context…": "正在检索上下文…",
  "Intent match in progress": "意图匹配进行中",
  "Semantic search in progress": "语义搜索进行中",
  "The Router is retrieving and reviewing bounded candidates.": "Router 正在受限预算内检索并审阅候选。",
  "Finding independently relevant pages.": "正在查找独立相关的页面。",
  "The previous completed context pack stays visible until this atomic request completes.": "在这次原子请求完成前，上一份完整 Context Pack 会继续保留在页面上。",
  "Ranking and budget assembly return as one stable context pack; unstable intermediate ranks are not shown.": "排序与预算组装会一次性返回稳定的 Context Pack；不会展示尚未稳定的中间排名。",
  "seconds": "秒",
  "Context pack review": "Context Pack 审阅",
  "Run a query to inspect the ranked context pack.": "执行查询以审阅按相关度排序的 Context Pack。",
  "Semantic retrieval returns independently relevant pages; asserted structure only makes bounded ranking adjustments.": "语义检索只返回独立相关的页面；已断言结构仅作受限排序加权。",
  "Intent matching lets the Router expand and review bounded candidates before it assembles a context pack.": "意图匹配允许 Router 在受限预算内扩展并审阅候选，再组装 Context Pack。",
  "Router selection": "Router 审阅选中",
  "Intent match audit": "意图匹配审计",
  "Router rounds": "Router 轮次",
  "Router token usage": "Router Token 用量",
  "Provider did not report token usage.": "提供方未报告 Token 用量。",
  "Input tokens": "输入 Token",
  "Output tokens": "输出 Token",
  "Cached tokens": "缓存 Token",
  "Reasoning tokens": "推理 Token",
  "Reported responses": "已报告响应",
  "tokens": "Token",
  "Candidates": "候选项",
  "Consulted Pages": "已 Consult 页面",
  "Relation leads reviewed": "已审阅关系线索",
  "Catalog Pages considered": "已检查目录页面",
  "Stopped": "停止原因",
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
  "Vector similarity": "向量相似度",
  "Structure boost": "结构加权",
  "via": "通过",
  "vector documents": "条向量文档",
  "new": "新建",
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
  "In progress": "进行中",
  "Selected": "已选",
  "Ready to continue": "可继续",
  "Scan complete": "扫描完成",
  "Scan candidates": "扫描候选",
  "Review and apply": "审阅应用",
  "Analyze Pack": "分析打包",
  "Analyze Summary": "分析摘要",
  "Extract Topic Page": "凝练新页",
  "Topic Page": "主题页",
  "Topic proposal": "凝练新页提案",
  "Analyze Relations": "分析关联",
  "Apply selected": "应用所选",
  "Continue to Summary": "继续到摘要",
  "Continue to Relations": "继续到关联",
  "Continue to Topic Page extraction": "继续到凝练新页",
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
  "Relation evidence": "关联理由",
  "Summary proposal": "摘要提案",
  "Diagnostics are separate from the current maintenance step and do not start model work.": "诊断与当前维护步骤分离，不会发起模型工作。",
  "Current stage": "当前阶段",
  "Analysis completed. No changes are recommended for this stage. Continue when you are ready.": "分析完成：本阶段没有需要优化的内容。准备好后可继续。",
  "Review the proposals below, select the changes to apply, then continue to the next stage.": "复核下方提案，选择要应用的变更，再继续到下一阶段。",
  "End this maintenance session?": "结束本次维护会话？",
  "No additional Page writes will occur. Unapplied proposals remain unapplied.": "不会再写入页面；未应用的提案将保持未应用。",
  "Maintenance session started": "维护会话已开始",
  "Maintenance session completed": "维护会话已完成",
  "Maintenance proceeds from Pack boundaries, through summaries and relations, to Topic Pages on the refreshed inventory. Each stage scans candidates, analyzes suggestions, then waits for your explicit application.": "维护依次处理 Pack 边界、摘要与关联，最后在刷新后的库存上凝练新页。每一段都会扫描候选、分析建议，并等待你明确应用。",
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
  "Observed client activity, latency, and telemetry coverage.": "客户端活动、延迟与遥测覆盖。",
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

function readSessionValue(key) {
  try { return window.sessionStorage.getItem(key); } catch (_) { return null; }
}

function writeSessionValue(key, value) {
  try { window.sessionStorage.setItem(key, value); } catch (_) {}
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
  governance: {
    loaded: false,
    busy: false,
    status: "archived",
    scope: "",
    hits: [],
    cursor: null,
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
        topic: null,
        relation: null,
      },
    },
    scan: null,
    pass: "pack",
    workflowStage: "scan",
    phase: "pack",
    analyses: { pack: null, summary: null, topic: null, relation: null },
    pendingCandidates: [],
    selected: new Set(),
    // Manual relation decisions remain local to the current review session until
    // the operator explicitly applies the selection.
    relationDraftStates: new Map(),
    relationReviewStates: new Map(),
    candidateReviewStates: new Map(),
    applyStates: new Map(),
    relationReviews: [],
    reviewBusy: new Set(),
    reviewDecisions: restoreReviewDecisions(readSessionValue(REVIEW_SESSION_STORAGE_KEY)),
    reviewCommitBusy: false,
    convergence: {
      running: false,
      report: null,
      steps: 0,
      completedAt: null,
      error: null,
    },
  },
  archive: {
    state: "idle",
    busy: false,
    activity: null,
    operation: null,
    scan: null,
    analyses: [],
    selected: new Set(),
    decisions: new Map(),
    applied: 0,
    retained: 0,
    deferred: 0,
    completedAt: null,
    issue: null,
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

function strokeIcon(paths, className = "") {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  if (className) svg.setAttribute("class", className);
  for (const d of paths) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

function openPageIcon() {
  return strokeIcon(["M5 19 19 5M8 5h11v11"]);
}

function relationCompareIcon() {
  return strokeIcon(["M4 5h6v14H4z", "M14 5h6v14h-6z", "M10 12h4", "m2-2 2 2-2 2"]);
}

function suppressRelationIcon() {
  return strokeIcon([
    "m9 15-1.4 1.4a3 3 0 0 1-4.2-4.2l3.5-3.5a3 3 0 0 1 4.2 0L12.5 10",
    "m15 9 1.4-1.4a3 3 0 0 1 4.2 4.2l-3.5 3.5a3 3 0 0 1-4.2 0L11.5 14",
    "M4 4 20 20",
  ]);
}

function acceptRelationIcon() {
  return strokeIcon(["M5 12.5 9.5 17 19 7"]);
}

function rejectRelationIcon() {
  return strokeIcon([
    "m8.5 15.5-1.6 1.6a3.5 3.5 0 0 1-5-5l3.2-3.2a3.5 3.5 0 0 1 4.5-.4",
    "m15.5 8.5 1.6-1.6a3.5 3.5 0 0 1 5 5l-3.2 3.2a3.5 3.5 0 0 1-4.5.4",
    "M8.8 12h6.4",
  ]);
}

function skipRelationIcon() {
  return strokeIcon(["M12 7v5l3 2", "M12 21a9 9 0 1 0-9-9"]);
}

function undoIcon() {
  return strokeIcon(["M9 7 4 12l5 5", "M5 12h8a6 6 0 0 1 6 6"]);
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

function formatInterval(seconds) {
  const value = Math.max(0, Number(seconds) || 0);
  if (value >= 3600) return `${Math.round(value / 3600)}h`;
  if (value >= 60) return `${Math.round(value / 60)}m`;
  return `${Math.round(value)}s`;
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

async function governanceMutation(path, body) {
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
    const reject = element("button", "secondary-button", "Reject");
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
  openPageIcon,
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

function metric(label, value, tone = "", note = "") {
  const node = element("div", `metric${tone ? ` tone-${tone}` : ""}`);
  node.append(element("div", "metric-label", label), element("div", "metric-value", value));
  if (note) node.append(element("div", "metric-note", note));
  return node;
}

function protocolMetric(version) {
  const node = element("div", "metric protocol-metric");
  node.append(element("div", "metric-label", t("Protocol")));
  const value = element("div", "metric-value");
  const raw = String(version || "-");
  const separator = raw.indexOf("-");
  if (separator < 1 || separator === raw.length - 1) {
    value.textContent = raw;
  } else {
    value.append(
      element("span", "protocol-version-number", raw.slice(0, separator)),
      element("span", "protocol-version-channel", raw.slice(separator + 1)),
    );
    value.title = raw;
    value.setAttribute("aria-label", raw);
  }
  node.append(value);
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
    protocolMetric(data.capabilities.protocolVersion),
    metric(t("Runtime PID"), data.runtime.pid || "-"),
    metric(t("Runtime started"), formatTime(data.runtime.startedAtUnixMs)),
  );

  byId("scope-rows").replaceChildren(...orderedScopes([...data.scopes]).map(({ scope, depth }) => {
    const row = document.createElement("tr");
    const open = element("button", "icon-button");
    open.type = "button";
    open.title = t("Open");
    open.setAttribute("aria-label", t("Open"));
    open.append(openPageIcon());
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
    if (scope.description) {
      const description = element("span", "scope-description", scope.description);
      description.title = scope.description;
      scopeCell.append(description);
    }
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
  renderGovernanceScopeOptions(data.scopes);
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
  const indicator = element("span", "page-open-indicator");
  indicator.append(openPageIcon());
  open.append(element("span", "page-title", pageSnippet(hit)), indicator);
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
  if (hit.sourceSpan) tags.push([["M5 7h11", "m-3-3 3 3-3 3", "M19 17H8", "m3 3-3-3 3-3"], t("Source stream")]);
  if (hit.summaryRevisionId) tags.push([["M5 7h14", "M5 12h10", "M5 17h7"], t("Summary route")]);
  if (hit.previewPayload?.mediaType === "application/vnd.pcp.packed-page+json") tags.push([["M4 5h16v14H4z", "M8 9h8", "M8 13h8", "M8 17h5"], t("Packed")]);
  return tags;
}

function pageRelationSignal(hit) {
  const stats = hit.relationStats;
  const signal = element("span", `page-signal${stats?.total ? "" : " page-signal-empty"}`);
  if (!stats) {
    signal.title = t("Unavailable");
    signal.append(strokeIcon(["M8 8h8", "m-2-2 2 2-2 2", "M16 16H8", "m2 2-2-2 2-2"], "page-meta-icon"), element("span", "", "–"));
    return signal;
  }
  signal.title = stats.total > 0
    ? `${formatNumber(stats.total)} ${t("Direct links")} · ${formatNumber(stats.incoming)} ${t("in")} · ${formatNumber(stats.outgoing)} ${t("out")}`
    : t("No direct links");
  signal.append(
    strokeIcon(["M8 8h8", "m-2-2 2 2-2 2", "M16 16H8", "m2 2-2-2 2-2"], "page-meta-icon"),
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
  meta.append(...pageStructureTags(hit).map(([paths, label]) => {
    const tag = element("span", "page-structure-tag");
    tag.title = label;
    tag.append(strokeIcon(paths, "page-meta-icon"), document.createTextNode(label));
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

function renderGovernanceScopeOptions(scopes = state.overview?.scopes || []) {
  const select = byId("governance-scope");
  if (!select) return;
  const selected = state.governance.scope;
  select.replaceChildren(
    new Option(t("All authorized scopes"), ""),
    ...orderedScopes([...scopes]).map(({ scope, depth }) => (
      new Option(`${"  ".repeat(depth)}${scope.displayName || scope.namespace}`, scope.namespace)
    )),
  );
  select.value = selected;
}

function governanceMeta(hit) {
  const meta = element("div", "governance-page-meta");
  meta.append(
    element("span", "", `${t("Kind")}: ${hit.kind}`),
    element("span", "", `${t("Scope")}: ${scopeName(hit.namespace)}`),
    element("span", "", `${t("Observed")}: ${formatTime(hit.observedAt || hit.createdAt)}`),
  );
  return meta;
}

function governanceCard(hit) {
  const archived = state.governance.status === "archived";
  const card = element("article", "governance-page");
  const heading = element("div", "governance-page-heading");
  const copy = element("div", "governance-page-copy");
  copy.append(element("strong", "governance-page-title", pageSnippet(hit)), governanceMeta(hit));
  const open = element("button", "icon-button");
  open.type = "button";
  open.title = t("Open in inspector");
  open.setAttribute("aria-label", t("Open in inspector"));
  open.append(openPageIcon());
  open.addEventListener("click", () => pageInspector.open(hit.pageId));
  heading.append(copy, open);

  const reason = document.createElement("input");
  reason.type = "text";
  reason.maxLength = 1200;
  reason.className = "governance-reason";
  reason.placeholder = archived ? t("Restore reason") : t("Archive reason");
  reason.setAttribute("aria-label", reason.placeholder);

  const action = element("button", archived ? "compact-button secondary-button" : "compact-button warning-button", archived ? t("Restore") : t("Archive"));
  action.type = "button";
  action.addEventListener("click", async () => {
    const reasonText = reason.value.trim();
    if (!reasonText) {
      reason.focus();
      showError(new Error(archived ? t("A restore reason is required") : t("An archive reason is required")));
      return;
    }
    const title = archived ? t("Restore this Page?") : t("Archive this Page?");
    const description = archived
      ? t("Restoring makes this Page eligible for default retrieval and graph expansion again.")
      : t("Archiving excludes this Page from default retrieval, graph expansion, and maintenance without deleting it.");
    if (!await confirmAction({ title, description, confirmLabel: archived ? t("Restore") : t("Archive") })) return;
    action.disabled = true;
    try {
      await governanceMutation(archived ? "/api/governance/restore" : "/api/governance/archive", {
        pageId: hit.pageId,
        expectedRevisionId: hit.revisionId,
        reason: reasonText,
      });
      await loadGovernance({ reload: true });
    } catch (error) {
      action.disabled = false;
      showError(error);
    }
  });
  const actions = element("div", "governance-page-actions");
  actions.append(reason, action);
  card.append(heading, actions);
  return card;
}

function renderGovernance(data, { append = false } = {}) {
  const hits = data.hits || [];
  state.governance.hits = append ? state.governance.hits.concat(hits) : hits;
  state.governance.cursor = data.nextCursor || null;
  state.governance.loaded = true;
  byId("governance-status").textContent = `${scopeName(state.governance.scope)} · ${t(state.governance.status === "archived" ? "Archived Pages" : "Active Pages")}`;
  byId("governance-archived")?.classList.toggle("active", state.governance.status === "archived");
  byId("governance-more").hidden = !state.governance.cursor;
  byId("governance-results").replaceChildren(
    ...(state.governance.hits.length
      ? state.governance.hits.map(governanceCard)
      : [element("div", "empty", t("No pages"))]),
  );
}

async function loadGovernance({ append = false, reload = false } = {}) {
  if (state.governance.busy) return;
  if (!reload && !append && state.governance.loaded) return;
  state.governance.busy = true;
  byId("governance-status").textContent = append ? t("Loading more") : t("Loading");
  byId("governance-more").disabled = true;
  try {
    const params = new URLSearchParams({ status: state.governance.status, limit: String(DEFAULT_PAGE_LIMIT) });
    if (state.governance.scope) params.set("scope", state.governance.scope);
    if (append && state.governance.cursor) params.set("cursor", state.governance.cursor);
    renderGovernance(await api(`/api/governance/pages?${params}`), { append });
  } catch (error) {
    showError(error);
    byId("governance-status").textContent = t("Load failed");
  } finally {
    state.governance.busy = false;
    byId("governance-more").disabled = false;
  }
}

function resetGovernance({ status = state.governance.status, scope = state.governance.scope } = {}) {
  state.governance.status = status;
  state.governance.scope = scope;
  state.governance.loaded = false;
  state.governance.hits = [];
  state.governance.cursor = null;
}

function archiveSessionActive() {
  return state.archive.state === "active";
}

function archiveSessionComplete() {
  return state.archive.state === "complete";
}

function resetArchiveSession() {
  state.archive.state = "idle";
  state.archive.busy = false;
  state.archive.activity = null;
  state.archive.operation = null;
  state.archive.scan = null;
  state.archive.analyses = [];
  state.archive.selected.clear();
  state.archive.decisions.clear();
  state.archive.applied = 0;
  state.archive.retained = 0;
  state.archive.deferred = 0;
  state.archive.completedAt = null;
  state.archive.issue = null;
}

function archiveCandidates() {
  return state.archive.analyses
    .map(({ scanPage, analysis, applied, applyIssue }) => !applied && analysis?.candidate
      ? { ...analysis.candidate, scanPage, applyIssue }
      : null)
    .filter(Boolean);
}

function refreshArchiveAnalysisTotals() {
  state.archive.retained = state.archive.analyses.filter(({ analysis }) => analysis?.decision === "retain").length;
  state.archive.deferred = state.archive.analyses.filter(({ status, analysis }) => (
    status === "failed" || analysis?.decision === "defer"
  )).length;
}

function renderArchiveSteps(stage, { scanning = false } = {}) {
  const states = {
    scan: [t("Completed"), t("Waiting"), t("Waiting")],
    analyze: [t("Completed"), t("In progress"), t("Waiting")],
    review: [t("Completed"), t("Completed"), t("Ready to apply")],
  };
  if (scanning) states.scan[0] = t("In progress");
  const steps = ["scan", "analyze", "review"];
  steps.forEach((name, index) => {
    const node = byId(`archive-step-${name}`);
    node.classList.toggle("active", name === stage);
    node.classList.toggle("completed", !scanning && index < steps.indexOf(stage));
    byId(`archive-step-${name}-status`).textContent = states[stage][index];
  });
}

function archiveProposalCard(candidate) {
  const decision = state.archive.decisions.get(candidate.candidateId);
  if (decision) {
    const row = element("article", `maintenance-review-settled-row tone-${decision === "archive" ? "reject" : "defer"}`);
    const stateNode = element("span", "maintenance-review-settled-state");
    stateNode.append(
      decision === "archive" ? rejectRelationIcon() : skipRelationIcon(),
      element("strong", "", decision === "archive" ? t("Archive") : t("Retained")),
    );
    const summary = element("span", "maintenance-review-settled-summary", compactRelationReviewPreview(candidate.preview || candidate.pageId, 180));
    summary.title = summary.textContent;
    const record = state.archive.analyses.find(({ analysis }) => analysis?.candidate?.candidateId === candidate.candidateId);
    const pending = element(
      "span",
      `maintenance-review-settled-pending${record?.applyIssue ? " has-error" : ""}`,
      record?.applyIssue
        ? (currentLanguage === "zh" ? "应用失败，可撤销或重试" : "Apply failed; undo or retry")
        : t("Pending commit"),
    );
    if (record?.applyIssue) pending.title = record.applyIssue;
    const undo = element("button", "compact-button maintenance-review-undo", t("Undo"));
    undo.type = "button";
    undo.prepend(undoIcon());
    undo.disabled = state.archive.operation === "apply";
    undo.addEventListener("click", () => {
      state.archive.selected.delete(candidate.candidateId);
      state.archive.decisions.delete(candidate.candidateId);
      renderArchiveSession();
    });
    row.append(
      element("span", "maintenance-review-kind maintenance-review-kind-archive", t("Archive")),
      stateNode,
      summary,
      pending,
      undo,
    );
    return row;
  }
  const card = element("article", "archive-candidate-card");
  const heading = element("div", "archive-candidate-heading");
  const copy = element("div", "archive-candidate-copy");
  copy.append(
    element("strong", "", `${candidate.kind} · ${scopeName(candidate.namespace)}`),
    element("span", "muted", `${formatTime(candidate.observedAt)} · ${formatSize(candidate.contentChars)}`),
  );
  const open = element("button", "icon-button");
  open.type = "button";
  open.title = t("Open in inspector");
  open.setAttribute("aria-label", t("Open in inspector"));
  open.append(openPageIcon());
  open.addEventListener("click", () => pageInspector.open(candidate.pageId));
  heading.append(copy, open);
  const preview = element("p", "archive-candidate-preview", candidate.preview);
  const reason = element("p", "archive-candidate-reason", candidate.reason);
  const signals = element("div", "archive-candidate-signals");
  candidate.candidateSignals.forEach((signal) => signals.append(element("span", "status-pill", signal)));
  if (candidate.applyIssue) {
    const failed = element("span", "maintenance-apply-state failed", currentLanguage === "zh" ? "应用失败，可重试" : "Apply failed; retry available");
    failed.title = candidate.applyIssue;
    signals.append(failed);
  }
  const actions = element("div", "maintenance-proposal-actions");
  const archive = element("button", "compact-button maintenance-proposal-reject", t("Archive"));
  archive.type = "button";
  archive.prepend(rejectRelationIcon());
  archive.addEventListener("click", () => {
    state.archive.selected.add(candidate.candidateId);
    state.archive.decisions.set(candidate.candidateId, "archive");
    renderArchiveSession();
  });
  const retain = element("button", "compact-button maintenance-proposal-skip", t("Retained"));
  retain.type = "button";
  retain.prepend(skipRelationIcon());
  retain.addEventListener("click", () => {
    state.archive.selected.delete(candidate.candidateId);
    state.archive.decisions.set(candidate.candidateId, "retain");
    renderArchiveSession();
  });
  actions.append(archive, retain);
  card.append(heading, preview, reason, signals, actions);
  return card;
}

function renderArchiveWorkflow() {
  const scan = state.archive.scan;
  const total = scan?.pages?.length || 0;
  const progress = batchProgress(state.archive.analyses);
  const analyzed = progress.processed;
  const attempts = state.archive.analyses.reduce((total, batch) => total + (batch.attempts || 0), 0);
  const candidates = archiveCandidates();
  const stage = !scan ? "scan" : analyzed < total ? "analyze" : "review";
  const scanning = state.archive.busy && !scan;
  const selectedCount = candidates.filter((candidate) => state.archive.selected.has(candidate.candidateId)).length;
  const reviewedCount = candidates.filter((candidate) => state.archive.decisions.has(candidate.candidateId)).length;
  renderArchiveSteps(stage, { scanning });
  byId("archive-scan-metrics").replaceChildren(
    metric(t("Candidates"), scanning ? "—" : formatNumber(total)),
    metric(t("Eligible Pages"), scanning ? "—" : formatNumber(scan?.eligiblePages || 0)),
    metric(t("Model calls"), `${formatNumber(attempts)} / ${formatNumber(scan?.estimatedModelCalls || 0)}`, attempts ? "info" : ""),
    metric(t("Archive proposals"), formatNumber(candidates.length), candidates.length ? "warning" : "", analyzed ? `${formatNumber(state.archive.retained)} ${t("Retained")} · ${formatNumber(state.archive.deferred)} ${t("Deferred")}` : ""),
  );
  const issue = byId("archive-issue");
  const batchIssues = state.archive.analyses
    .filter((batch) => batch.issue || batch.applyIssue)
    .map((batch) => `${currentLanguage === "zh" ? "页面" : "Page"} ${formatNumber(batch.batchIndex + 1)}: ${batch.applyIssue || batch.issue}`);
  const issueText = [state.archive.issue, ...batchIssues].filter(Boolean).join("\n");
  issue.hidden = !issueText;
  issue.textContent = issueText;
  byId("archive-status").textContent = state.archive.busy
    ? state.archive.activity || t("Working")
    : stage === "review"
      ? `${formatNumber(candidates.length)} ${t("Archive proposals")}`
      : scanning
        ? t("Scanning candidates")
        : `${formatNumber(analyzed)} / ${formatNumber(total)} ${t("Analyzed")}`;
  const analyze = byId("archive-analyze");
  analyze.disabled = state.archive.busy || !scan || !state.archive.analyses.some((batch) => batch.status === "pending");
  analyze.textContent = analyzed ? t("Continue analysis") : t("Analyze suggestions");
  const apply = byId("archive-apply");
  apply.disabled = state.archive.busy || stage !== "review" || selectedCount === 0;
  apply.textContent = `${t("Archive selected")} (${formatNumber(selectedCount)})`;
  const rescan = byId("archive-rescan");
  rescan.disabled = state.archive.busy;
  const finish = byId("archive-finish");
  finish.disabled = state.archive.busy || stage !== "review";
  const retry = byId("archive-retry-failed");
  retry.hidden = progress.failed === 0;
  retry.disabled = state.archive.busy || progress.failed === 0;
  retry.textContent = `${t("Retry failed batches")} (${formatNumber(progress.failed)})`;
  const analysisLog = byId("archive-analysis-log");
  analysisLog.hidden = !state.archive.analyses.length;
  byId("archive-analysis-log-summary").textContent = currentLanguage === "zh"
    ? `分析进度：${formatNumber(progress.completed)} 个完成，${formatNumber(progress.failed)} 个失败，共 ${formatNumber(progress.total)} 个。`
    : `Analysis progress: ${formatNumber(progress.completed)} complete, ${formatNumber(progress.failed)} failed, ${formatNumber(progress.total)} total.`;
  byId("archive-analysis-log-body").textContent = state.archive.analyses
    .filter((batch) => batch.status !== "pending" || batch.issue)
    .map((batch) => `${currentLanguage === "zh" ? "页面" : "Page"} ${formatNumber(batch.batchIndex + 1)} · ${batch.status}${batch.analysis?.decision ? ` · ${batch.analysis.decision}` : ""}${batch.issue ? ` · ${batch.issue}` : ""}${batch.applyIssue ? ` · ${batch.applyIssue}` : ""}`)
    .join("\n");
  const proposals = byId("archive-proposals");
  proposals.hidden = !["analyze", "review"].includes(stage) || candidates.length === 0;
  proposals.classList.toggle("is-live", stage === "analyze");
  if (!proposals.hidden) {
    byId("archive-selection-status").textContent = stage === "analyze"
      ? (currentLanguage === "zh"
        ? `分析仍在继续 · 已审 ${formatNumber(reviewedCount)} / ${formatNumber(candidates.length)} · 完成前不能应用`
        : `Analysis is still running · ${formatNumber(reviewedCount)} of ${formatNumber(candidates.length)} reviewed · applying is locked`)
      : (currentLanguage === "zh"
        ? `已审 ${formatNumber(reviewedCount)} / ${formatNumber(candidates.length)} · ${formatNumber(candidates.length - reviewedCount)} 待定`
        : `${formatNumber(reviewedCount)} of ${formatNumber(candidates.length)} reviewed · ${formatNumber(candidates.length - reviewedCount)} remaining`);
    byId("archive-cards").replaceChildren(
      ...(candidates.length ? candidates.map(archiveProposalCard) : [element("div", "empty", t("No archive proposals"))]),
    );
  }
}

function renderArchiveReport() {
  byId("archive-status").textContent = t("Archive review complete");
  byId("archive-report-status").textContent = formatTime(state.archive.completedAt);
  byId("archive-report-metrics").replaceChildren(
    metric(t("Archived"), formatNumber(state.archive.applied), state.archive.applied ? "positive" : ""),
    metric(t("Retained"), formatNumber(state.archive.retained)),
    metric(t("Deferred"), formatNumber(state.archive.deferred), state.archive.deferred ? "warning" : ""),
  );
}

function renderArchiveSession() {
  byId("archive-idle").hidden = !(!archiveSessionActive() && !archiveSessionComplete());
  byId("archive-workflow").hidden = !archiveSessionActive();
  byId("archive-report").hidden = !archiveSessionComplete();
  byId("archive-start").disabled = state.archive.busy || maintenanceSessionActive();
  if (!archiveSessionActive() && !archiveSessionComplete()) {
    byId("archive-status").textContent = maintenanceSessionActive() ? t("Available after maintenance") : "";
  }
  if (archiveSessionActive()) renderArchiveWorkflow();
  if (archiveSessionComplete()) renderArchiveReport();
}

async function scanArchiveCandidates() {
  if (state.archive.busy) return;
  state.archive.busy = true;
  state.archive.activity = t("Scanning candidates");
  state.archive.operation = "scan";
  state.archive.issue = null;
  renderArchiveSession();
  try {
    const scan = await maintenanceMutation("/api/maintenance/archive/scan", {});
    state.archive.scan = scan;
    state.archive.analyses = (scan.pages || []).map((scanPage, batchIndex) => ({
      batchIndex,
      scanPage,
      status: "pending",
      attempts: 0,
      analysis: null,
      issue: null,
      applyIssue: null,
      applied: false,
    }));
    state.archive.selected.clear();
    state.archive.decisions.clear();
    state.archive.retained = 0;
    state.archive.deferred = 0;
    if (!(scan.pages || []).length) {
      state.archive.state = "complete";
      state.archive.completedAt = new Date().toISOString();
    }
  } catch (error) {
    state.archive.issue = error.message || String(error);
    showError(error);
  } finally {
    state.archive.busy = false;
    state.archive.activity = null;
    state.archive.operation = null;
    renderArchiveSession();
  }
}

async function startArchiveSession() {
  if (state.archive.busy || maintenanceSessionActive()) return;
  resetArchiveSession();
  state.archive.state = "active";
  renderArchiveSession();
  await scanArchiveCandidates();
}

async function analyzeArchiveCandidates({ retryFailed = false } = {}) {
  const scan = state.archive.scan;
  if (!archiveSessionActive() || state.archive.busy || !scan) return;
  const indexes = runnableBatchIndexes(state.archive.analyses, { retryFailed });
  if (!indexes.length) return;
  state.archive.busy = true;
  state.archive.operation = "analyze";
  state.archive.issue = null;
  for (const [progressIndex, batchIndex] of indexes.entries()) {
    const batch = state.archive.analyses[batchIndex];
    if (!batch) continue;
    const page = batch.scanPage;
    beginBatch(batch);
    batch.applyIssue = null;
    state.archive.activity = `${t("Analyzing")} ${formatNumber(progressIndex + 1)} / ${formatNumber(indexes.length)}`;
    renderArchiveSession();
    try {
      const analysis = await maintenanceMutation("/api/maintenance/archive/analyze", {
        scanId: scan.scanId,
        pageId: page.pageId,
        revisionId: page.revisionId,
      });
      completeBatch(batch, { analysis });
    } catch (error) {
      failBatch(batch, error);
    }
    refreshArchiveAnalysisTotals();
    renderArchiveSession();
  }
  state.archive.busy = false;
  state.archive.activity = null;
  state.archive.operation = null;
  renderArchiveSession();
}

async function applyArchiveSelection() {
  const selected = archiveCandidates().filter((candidate) => state.archive.selected.has(candidate.candidateId));
  if (!selected.length || state.archive.busy) return;
  const confirmed = await confirmAction({
    title: t("Archive selected Pages?"),
    description: t("Archiving removes the selected Pages from default retrieval, graph expansion, and ordinary maintenance. It does not delete them; they remain available for direct review and restoration."),
    confirmLabel: t("Archive"),
  });
  if (!confirmed) return;
  state.archive.busy = true;
  state.archive.operation = "apply";
  state.archive.issue = null;
  const failures = [];
  for (const [index, candidate] of selected.entries()) {
    state.archive.activity = `${t("Archiving")} ${formatNumber(index + 1)} / ${formatNumber(selected.length)}`;
    renderArchiveSession();
    const record = state.archive.analyses.find(({ analysis }) => analysis?.candidate?.candidateId === candidate.candidateId);
    try {
      await maintenanceMutation("/api/maintenance/archive/apply", {
        pageId: candidate.pageId,
        expectedRevisionId: candidate.revisionId,
        reason: `${t("Human-approved archive review")}: ${candidate.reason}`,
      });
      state.archive.applied += 1;
      if (record) {
        record.applied = true;
        record.applyIssue = null;
      }
      state.archive.selected.delete(candidate.candidateId);
      state.archive.decisions.delete(candidate.candidateId);
    } catch (error) {
      const message = error.message || String(error);
      if (record) record.applyIssue = message;
      failures.push({ candidateId: candidate.candidateId, message });
    }
    renderArchiveSession();
  }
  try {
    resetGovernance({ status: "archived", scope: state.governance.scope });
  } finally {
    state.archive.busy = false;
    state.archive.activity = null;
    state.archive.operation = null;
    state.archive.issue = failures.length
      ? (currentLanguage === "zh"
        ? `${formatNumber(failures.length)} 个归档提案未能应用；成功项已保留，失败项仍可重试。`
        : `${formatNumber(failures.length)} archive proposals could not be applied. Successful items were kept and failed items remain available to retry.`)
      : null;
    renderArchiveSession();
  }
}

async function finishArchiveSession() {
  if (!archiveSessionActive() || state.archive.busy) return;
  const candidates = archiveCandidates();
  const unresolved = candidates.filter((candidate) => !state.archive.decisions.has(candidate.candidateId));
  if (unresolved.length) {
    const confirmed = await confirmAction({
      title: currentLanguage === "zh" ? "保留未审归档提案？" : "Retain unreviewed archive proposals?",
      description: currentLanguage === "zh"
        ? `还有 ${formatNumber(unresolved.length)} 个提案未审；结束后它们会保持原状，不会归档。`
        : `${formatNumber(unresolved.length)} proposals remain unreviewed. They will stay unchanged and will not be archived.`,
      confirmLabel: t("End session"),
    });
    if (!confirmed) return;
  }
  state.archive.retained += candidates.filter((candidate) => (
    state.archive.decisions.get(candidate.candidateId) === "retain"
      || !state.archive.decisions.has(candidate.candidateId)
  )).length;
  state.archive.state = "complete";
  state.archive.completedAt = new Date().toISOString();
  renderArchiveSession();
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
    rejected: 0,
    suppressed: 0,
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
      topic: emptyMaintenanceOutcome(),
      relation: emptyMaintenanceOutcome(),
    },
  };
  state.maintenance.pass = "pack";
  state.maintenance.workflowStage = "scan";
  state.maintenance.phase = "pack";
  state.maintenance.scan = null;
  state.maintenance.analyses = { pack: null, summary: null, topic: null, relation: null };
  state.maintenance.pendingCandidates = [];
  state.maintenance.selected.clear();
  state.maintenance.relationDraftStates.clear();
  state.maintenance.relationReviewStates.clear();
  state.maintenance.candidateReviewStates.clear();
  state.maintenance.applyStates.clear();
}

function renderMaintenanceStatus(status) {
  state.maintenance.status = status;
  state.maintenance.loaded = true;
  renderAutomationStatus();
  renderMaintenanceSession();
}

function maintenanceSceneError() {
  if (state.maintenance.convergence.error) return state.maintenance.convergence.error;
  if (state.maintenance.convergence.running) return null;
  if (state.maintenance.convergence.completedAt) return null;
  const message = state.maintenance.status?.automation?.lastError;
  return message ? { message } : null;
}

function maintenanceSceneErrorCopy(error) {
  const detail = error?.message || String(error || "");
  if (detail.includes("invalid relation pair")) {
    return {
      title: t("Relation analysis paused"),
      summary: t("The model returned a Page pair outside the current candidate window. No relation was applied for this work unit."),
      detail,
    };
  }
  return {
    title: t("Maintenance run paused"),
    summary: t("The current maintenance run stopped before convergence. Completed work and accumulated reviews are preserved."),
    detail,
  };
}

function renderMaintenanceConvergenceState() {
  const error = maintenanceSceneError();
  const phase = error
    ? "failed"
    : convergencePhase(state.maintenance.convergence, state.maintenance.relationReviews.length);
  const labels = {
    waiting: "Awaiting run",
    running: "Run in progress",
    review: "Awaiting review",
    settled: "Converged",
    failed: "Needs attention",
  };
  const tones = {
    waiting: "warning",
    running: "info",
    review: "warning",
    settled: "positive",
    failed: "warning",
  };
  const stateNode = byId("maintenance-convergence-state");
  stateNode.textContent = t(labels[phase]);
  stateNode.className = `status-pill status-${tones[phase]}`;

  const alert = byId("maintenance-scene-alert");
  alert.hidden = !error;
  byId("maintenance-scene-alert-retry").disabled = state.maintenance.busy || !maintenanceAvailable() || archiveSessionActive();
  if (!error) return;
  const copy = maintenanceSceneErrorCopy(error);
  byId("maintenance-scene-alert-title").textContent = copy.title;
  byId("maintenance-scene-alert-summary").textContent = copy.summary;
  byId("maintenance-scene-alert-detail").textContent = copy.detail;
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

function renderMaintenanceAutomationChart(status) {
  const automation = status.automation || {};
  const chart = byId("maintenance-automation-chart");
  const queueValues = [
    [t("Dirty regions"), automation.dirtyRegionCount || 0, "warning"],
    [t("Ready regions"), automation.readyRegionCount || 0, "accent"],
    [t("Pending review"), automation.pendingReviewCount || 0, "positive"],
  ];
  const queueMax = Math.max(1, ...queueValues.map(([, value]) => value));
  const queue = element("section", "maintenance-chart-card maintenance-queue-chart");
  queue.append(element("strong", "maintenance-chart-title", currentLanguage === "zh" ? "工作负载" : "Workload"));
  for (const [label, value, tone] of queueValues) {
    const row = element("div", "maintenance-chart-row");
    const track = element("span", "maintenance-chart-track");
    const fill = element("span", `maintenance-chart-fill tone-${tone}`);
    fill.style.setProperty("--chart-value", value ? `${Math.max(3, (value / queueMax) * 100)}%` : "0%");
    track.append(fill);
    row.append(element("span", "maintenance-chart-label", label), track, element("strong", "maintenance-chart-value", formatNumber(value)));
    queue.append(row);
  }

  const baseInterval = Math.max(1, status.intervalSeconds || 1);
  const maxInterval = Math.max(baseInterval, status.maxIntervalSeconds || baseInterval);
  const scheduledInterval = Math.min(
    maxInterval,
    baseInterval * (2 ** Math.min(Math.max(0, (automation.idleCycles || 0) - 1), 20)),
  );
  const cadence = element("section", "maintenance-chart-card maintenance-cadence-chart");
  const ring = element("div", "maintenance-cadence-ring");
  ring.style.setProperty("--chart-progress", `${Math.max(4, (scheduledInterval / maxInterval) * 100)}%`);
  ring.append(
    element("strong", "", formatInterval(scheduledInterval)),
    element("span", "", currentLanguage === "zh" ? "当前间隔" : "current"),
  );
  const cadenceCopy = element("div", "maintenance-cadence-copy");
  cadenceCopy.append(
    element("strong", "maintenance-chart-title", currentLanguage === "zh" ? "自适应节奏" : "Adaptive cadence"),
    element("span", "muted", currentLanguage === "zh"
      ? `基础 ${formatInterval(baseInterval)} · 上限 ${formatInterval(maxInterval)}`
      : `Base ${formatInterval(baseInterval)} · ceiling ${formatInterval(maxInterval)}`),
    element("span", "muted", t("Write activity wakes Runtime early.")),
  );
  cadence.append(ring, cadenceCopy);
  chart.replaceChildren(queue, cadence);
}

function renderAutomationStatus() {
  const status = state.maintenance.status;
  const section = byId("maintenance-automation-status");
  section.hidden = !status?.available;
  if (section.hidden) return;
  const automation = status.automation || {};
  const stateNode = byId("maintenance-automation-state");
  stateNode.textContent = automationStateLabel(status);
  stateNode.className = `status-pill status-${automationStateTone(status)}`;
  byId("maintenance-automation-metrics").replaceChildren(
    metric(
      t("Maintenance inventory"),
      formatNumber(automation.observedPageCount),
      "",
      t("Includes retained superseded Pages for maintenance review."),
    ),
    metric(t("Dirty regions"), formatNumber(automation.dirtyRegionCount), automation.dirtyRegionCount ? "warning" : ""),
    metric(t("Ready regions"), formatNumber(automation.readyRegionCount), automation.readyRegionCount ? "info" : ""),
    metric(t("Pending review"), formatNumber(automation.pendingReviewCount), automation.pendingReviewCount ? "warning" : ""),
  );
  renderMaintenanceAutomationChart(status);
  const convergence = state.maintenance.convergence;
  const currentReport = convergence.running
    ? convergence.report
    : automation.currentReport || convergence.report;
  const progress = byId("maintenance-automation-progress");
  progress.hidden = !currentReport;
  if (currentReport) {
    const proposed = (currentReport.summariesProposed || 0)
      + (currentReport.packsProposed || 0)
      + (currentReport.relationsProposed || 0)
      + (currentReport.topicsProposed || 0)
      + (currentReport.archivesProposed || 0)
      + (currentReport.retentionLeasesProposed || 0);
    const committed = (currentReport.summariesWritten || 0)
      + (currentReport.packsCommitted || 0)
      + (currentReport.relationsCommitted || 0)
      + (currentReport.retentionLeasesWritten || 0);
    byId("maintenance-automation-progress-title").textContent = convergence.running
      ? (currentLanguage === "zh" ? `立即运行 · 已推进 ${formatNumber(convergence.steps)} 个工作单元` : `Run now · ${formatNumber(convergence.steps)} bounded jobs advanced`)
      : automation.state === "running"
      ? (currentLanguage === "zh" ? "本轮实时进度" : "Live cycle progress")
      : convergence.completedAt
        ? t("Maintenance converged")
        : (currentLanguage === "zh" ? "上次失败前的部分进度" : "Partial progress before the last failure");
    byId("maintenance-automation-progress-metrics").replaceChildren(
      metric(t("Maintenance inventory"), formatNumber(currentReport.inspectedPages)),
      metric(t("Model calls"), formatNumber(currentReport.workerCalls), currentReport.workerCalls ? "info" : ""),
      metric(t("Proposals"), formatNumber(proposed), proposed ? "warning" : ""),
      metric(t("Applied"), formatNumber(committed), committed ? "positive" : "", `${formatNumber(currentReport.deferred || 0)} ${t("Deferred")} · ${formatNumber(currentReport.escalatedDecisions || 0)} ${t("Model escalation")}`),
    );
  }
  const completed = automation.lastCompletedAt;
  const started = automation.lastStartedAt;
  const nextWake = automation.nextWakeAt;
  byId("maintenance-automation-detail").textContent = started || completed || nextWake
    ? [
        started ? `${currentLanguage === "zh" ? "最近开始" : "Last started"}: ${formatTime(started)}` : "",
        completed ? `${t("Last completed")}: ${formatTime(completed)}` : "",
        nextWake ? `${t("Next automatic check")}: ${formatTime(nextWake)}` : "",
        automation.idleCycles ? `${t("Idle backoff")}: ${formatNumber(automation.idleCycles)}` : "",
        automation.consecutiveFailures ? `${t("Consecutive failures")}: ${formatNumber(automation.consecutiveFailures)}` : "",
        t("Write activity wakes Runtime early."),
      ].filter(Boolean).join(" · ")
    : t("Awaiting the first Runtime heartbeat.");
  const error = byId("maintenance-automation-error");
  // Operation failures are owned by the scene-level alert above the workflow.
  // Keeping the old footer hidden avoids two competing error locations.
  error.hidden = true;
  error.textContent = "";
  renderMaintenanceConvergenceState();
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

function relationComparisonButton(proposal, className = "secondary-button", {
  iconOnly = false,
  onAccept = null,
  onReject = null,
  onSkip = null,
  accepted = false,
  rejected = false,
  skipped = false,
} = {}) {
  const label = t("Compare Pages");
  const compare = element("button", className, iconOnly ? undefined : label);
  compare.type = "button";
  if (iconOnly) compare.append(relationCompareIcon());
  compare.title = label;
  compare.setAttribute("aria-label", label);
  compare.addEventListener("click", () => {
    pageInspector.compareRelation({
      pages: proposal.pages,
      relationReason: proposal.relationReason,
      reviewReason: proposal.reviewReason,
      onAccept,
      onReject,
      onSkip,
      accepted,
      rejected,
      skipped,
    }).catch(() => {});
  });
  return compare;
}

function topicExtractionReviewButton(candidate, className = "secondary-button") {
  const review = element("button", className, t("Review source Pages"));
  review.type = "button";
  review.addEventListener("click", () => {
    pageInspector.reviewTopic(candidate).catch(() => {});
  });
  return review;
}

function renderRelationReviews() {
  const section = byId("maintenance-relation-review");
  const reviews = state.maintenance.relationReviews;
  section.hidden = !maintenanceAvailable();
  if (section.hidden) return;
  const { pending, staged } = partitionReviewSession(reviews, state.maintenance.reviewDecisions);
  persistMaintenanceReviewSession();
  const total = reviews.length;
  const stagedCount = staged.length;
  byId("maintenance-relation-review-count").textContent = currentLanguage === "zh"
    ? `${formatNumber(pending.length)} 待处理 · ${formatNumber(stagedCount)} 待提交`
    : `${formatNumber(pending.length)} remaining · ${formatNumber(stagedCount)} staged`;

  const progress = byId("maintenance-review-progress");
  progress.hidden = total === 0;
  if (total) {
    const ring = element("div", "maintenance-review-progress-ring");
    ring.style.setProperty("--chart-progress", `${(stagedCount / total) * 100}%`);
    ring.append(element("strong", "", `${formatNumber(stagedCount)}/${formatNumber(total)}`), element("span", "", t("Decided")));
    const copy = element("div", "maintenance-review-progress-copy");
    copy.append(
      element("strong", "", t("Review progress")),
      element("span", "muted", `${t("Remaining")} ${formatNumber(pending.length)} · ${t("Pending commit")} ${formatNumber(stagedCount)}`),
    );
    const legend = element("div", "maintenance-review-progress-legend");
    const counts = reviewDecisionCounts(state.maintenance.reviewDecisions);
    [
      ["accept", t("Accepted for this review")],
      ["reject", t("Rejected for this review")],
      ["defer", t("Deferred for now")],
      ["suppress", t("Will be suppressed")],
    ].forEach(([decision, label]) => {
      if (counts[decision]) legend.append(element("span", `tone-${decision}`, `${label} ${formatNumber(counts[decision])}`));
    });
    copy.append(legend);
    progress.replaceChildren(ring, copy);
  }

  byId("maintenance-relation-review-cards").replaceChildren(
    ...(pending.length
      ? pending.map(maintenanceReviewCard)
      : total
        ? []
        : [element("div", "maintenance-review-empty", t("Review inbox is clear"))]),
  );
  byId("maintenance-review-settled").replaceChildren(
    ...staged.map(({ review, decision }) => maintenanceReviewSettledRow(review, decision)),
  );
  const sessionActions = byId("maintenance-review-session-actions");
  sessionActions.hidden = stagedCount === 0;
  if (stagedCount) {
    const counts = reviewDecisionCounts(state.maintenance.reviewDecisions);
    byId("maintenance-review-session-summary").textContent = currentLanguage === "zh"
      ? `${formatNumber(counts.total)} 个决定尚未写入；完成前均可撤销。`
      : `${formatNumber(counts.total)} decisions are not written yet and remain reversible until completion.`;
    byId("maintenance-review-commit").textContent = `${t("Finish review and apply")} (${formatNumber(counts.total)})`;
    byId("maintenance-review-commit").disabled = state.maintenance.reviewCommitBusy;
    byId("maintenance-review-undo-all").disabled = state.maintenance.reviewCommitBusy;
  }
}

function maintenanceReviewKindLabel(kind) {
  return {
    pack: "Pack",
    summary: "Summary",
    relation: "Relations",
    topic: "Extract Topic Page",
    archive: "Archive",
  }[kind] || kind;
}

function maintenanceReviewContent(review) {
  const payload = review.payload || {};
  const candidate = payload.candidate || {};
  const body = element("div", `maintenance-review-content maintenance-review-${payload.kind || "unknown"}`);
  if (payload.kind === "relation") {
    const pages = element("div", "maintenance-relation-review-pages");
    (candidate.pages || []).forEach((page, index) => {
      const pageCard = element("button", "maintenance-relation-review-page");
      pageCard.type = "button";
      pageCard.append(
        element("span", "mono muted", page.pageId),
        element("span", "maintenance-relation-review-preview", compactRelationReviewPreview(page.preview)),
        element("span", "mono muted", page.revisionId),
      );
      pageCard.addEventListener("click", () => pageInspector.compareRelation({
        pages: candidate.pages,
        relationReason: candidate.relationReason,
        reviewReason: review.reason,
      }).catch(() => {}));
      pages.append(pageCard);
      if (index === 0) pages.append(element("span", "maintenance-relation-review-link", "↔"));
    });
    if (candidate.relationReason) body.append(element("p", "maintenance-review-evidence", candidate.relationReason));
    body.append(pages);
  } else if (payload.kind === "topic") {
    body.append(
      element("strong", "maintenance-review-title", candidate.title),
      element("p", "maintenance-review-preview", candidate.content),
      element("span", "muted", `${formatNumber(candidate.pages?.length || 0)} ${t("Source Pages")}`),
    );
  } else if (payload.kind === "summary") {
    body.append(
      element("span", "mono muted", candidate.pageId),
      element("p", "maintenance-review-preview", candidate.content),
    );
  } else if (payload.kind === "pack") {
    body.append(element("span", "muted", `${formatNumber(candidate.inputPageCount || candidate.pages?.length || 0)} ${t("Pages")} · ${formatNumber(candidate.contentChars || 0)} chars`));
    const pages = element("div", "maintenance-review-source-list");
    (candidate.pages || []).slice(0, 4).forEach((page) => pages.append(
      element("p", "maintenance-review-source", compactRelationReviewPreview(page.preview)),
    ));
    body.append(pages);
  } else if (payload.kind === "archive") {
    body.append(
      element("span", "mono muted", candidate.pageId),
      element("p", "maintenance-review-preview", candidate.preview),
      element("p", "maintenance-review-evidence", candidate.reason),
    );
    const signals = element("div", "maintenance-review-signals");
    (candidate.candidateSignals || []).forEach((signal) => signals.append(element("span", "status-pill", signal)));
    body.append(signals);
  }
  return body;
}

function reviewDecisionLabel(decision) {
  return t({
    accept: "Accepted for this review",
    reject: "Rejected for this review",
    defer: "Deferred for now",
    suppress: "Will be suppressed",
  }[decision] || decision);
}

function maintenanceReviewSummary(review) {
  const candidate = review.payload?.candidate || {};
  if (candidate.title) return candidate.title;
  if (candidate.pageId) return candidate.pageId;
  if (candidate.pages?.length) {
    return candidate.pages.map((page) => page.pageId).join(" ↔ ");
  }
  return review.candidateId;
}

function maintenanceReviewSettledRow(review, staged) {
  const row = element("article", `maintenance-review-settled-row tone-${staged.decision}`);
  const stateNode = element("span", "maintenance-review-settled-state");
  stateNode.append(
    staged.decision === REVIEW_DECISION.ACCEPT ? acceptRelationIcon()
      : staged.decision === REVIEW_DECISION.DEFER ? skipRelationIcon()
        : staged.decision === REVIEW_DECISION.SUPPRESS ? suppressRelationIcon()
          : rejectRelationIcon(),
    element("strong", "", reviewDecisionLabel(staged.decision)),
  );
  const summary = element("span", "maintenance-review-settled-summary", maintenanceReviewSummary(review));
  summary.title = summary.textContent;
  const pending = element("span", "maintenance-review-settled-pending", staged.error || t("Pending commit"));
  if (staged.error) pending.classList.add("has-error");
  const undo = element("button", "compact-button maintenance-review-undo", t("Undo"));
  undo.type = "button";
  undo.prepend(undoIcon());
  undo.disabled = state.maintenance.reviewCommitBusy;
  undo.addEventListener("click", () => undoMaintenanceReview(review.candidateId));
  row.append(
    element("span", `maintenance-review-kind maintenance-review-kind-${review.payload?.kind || "unknown"}`, t(maintenanceReviewKindLabel(review.payload?.kind))),
    stateNode,
    summary,
    pending,
    undo,
  );
  return row;
}

function reviewDecisionButton(review, decision, label, icon, tone = "") {
  const button = element("button", `compact-button maintenance-review-decision${tone ? ` tone-${tone}` : ""}`);
  button.type = "button";
  button.disabled = state.maintenance.reviewCommitBusy;
  button.append(icon, element("span", "", label));
  button.addEventListener("click", () => stageMaintenanceReview(review, decision));
  return button;
}

function maintenanceReviewCard(review) {
  const payload = review.payload || {};
  const candidate = payload.candidate || {};
  const card = element("article", "maintenance-relation-review-card maintenance-review-card");
  const heading = element("div", "maintenance-relation-review-card-heading");
  const metadata = element("div", "maintenance-review-metadata");
  metadata.append(
    element("span", `maintenance-review-kind maintenance-review-kind-${payload.kind || "unknown"}`, t(maintenanceReviewKindLabel(payload.kind))),
    element("span", "muted", review.origin === "automatic" ? t("Automatic maintenance") : t("Manual maintenance")),
    element("span", "muted", formatTime(review.proposedAt)),
  );
  const model = element("span", `status-pill ${review.escalated ? "status-warning" : ""}`, review.escalated
    ? `${t("Escalated model")} · ${formatNumber(review.modelAttempts)}`
    : `${t("Baseline model")} · ${formatNumber(review.modelAttempts)}`);
  heading.append(metadata, model);
  const reason = element("p", "maintenance-relation-review-reason muted", review.reason);
  const actions = element("div", "maintenance-relation-review-actions");
  if (payload.kind === "relation") {
    actions.append(relationComparisonButton({
      pages: candidate.pages,
      relationReason: candidate.relationReason,
      reviewReason: review.reason,
    }, "compact-button compact-icon-button", {
      iconOnly: true,
      onAccept: () => stageMaintenanceReview(review, REVIEW_DECISION.ACCEPT),
      onReject: () => stageMaintenanceReview(review, REVIEW_DECISION.REJECT),
      onSkip: () => stageMaintenanceReview(review, REVIEW_DECISION.DEFER),
    }));
  } else if (payload.kind === "topic") {
    actions.append(topicExtractionReviewButton(candidate, "compact-button secondary-button"));
  }
  const accept = reviewDecisionButton(
    review,
    REVIEW_DECISION.ACCEPT,
    t(payload.kind === "archive" ? "Archive" : "Accept"),
    acceptRelationIcon(),
    payload.kind === "archive" ? "archive" : "accept",
  );
  const reject = reviewDecisionButton(review, REVIEW_DECISION.REJECT, t("Reject"), rejectRelationIcon(), "reject");
  const defer = reviewDecisionButton(review, REVIEW_DECISION.DEFER, t("Skip for now"), skipRelationIcon(), "defer");
  actions.append(accept, reject, defer);
  if (payload.kind === "relation") {
    const suppress = reviewDecisionButton(
      review,
      REVIEW_DECISION.SUPPRESS,
      t("Do not suggest this relation again"),
      suppressRelationIcon(),
      "suppress",
    );
    actions.append(suppress);
  }
  card.append(heading, reason, maintenanceReviewContent(review), actions);
  return card;
}

async function loadRelationReviews() {
  if (!maintenanceAvailable()) {
    state.maintenance.relationReviews = [];
    renderRelationReviews();
    return;
  }
  const response = await api("/api/maintenance/reviews");
  state.maintenance.relationReviews = response.reviews || [];
  renderRelationReviews();
}

function persistMaintenanceReviewSession() {
  writeSessionValue(
    REVIEW_SESSION_STORAGE_KEY,
    serializeReviewDecisions(state.maintenance.reviewDecisions),
  );
}

function stageMaintenanceReview(review, decision) {
  if (state.maintenance.reviewCommitBusy) return;
  stageReviewDecision(state.maintenance.reviewDecisions, review, decision);
  persistMaintenanceReviewSession();
  renderRelationReviews();
}

function undoMaintenanceReview(candidateId) {
  if (state.maintenance.reviewCommitBusy) return;
  undoReviewDecision(state.maintenance.reviewDecisions, candidateId);
  persistMaintenanceReviewSession();
  renderRelationReviews();
}

function undoAllMaintenanceReviews() {
  if (state.maintenance.reviewCommitBusy) return;
  state.maintenance.reviewDecisions.clear();
  persistMaintenanceReviewSession();
  renderRelationReviews();
}

async function commitMaintenanceReviewSession() {
  if (state.maintenance.reviewCommitBusy) return;
  const { staged } = partitionReviewSession(
    state.maintenance.relationReviews,
    state.maintenance.reviewDecisions,
  );
  if (!staged.length) return;
  const counts = reviewDecisionCounts(state.maintenance.reviewDecisions);
  const summary = currentLanguage === "zh"
    ? `接受 ${formatNumber(counts.accept)} · 拒绝 ${formatNumber(counts.reject)} · 延后 ${formatNumber(counts.defer)} · 不再建议 ${formatNumber(counts.suppress)}`
    : `${formatNumber(counts.accept)} accept · ${formatNumber(counts.reject)} reject · ${formatNumber(counts.defer)} defer · ${formatNumber(counts.suppress)} suppress`;
  const confirmed = await confirmAction({
    title: t("Finish this review session?"),
    description: `${t("The staged decisions below will now be applied. This is the point where they become persistent.")} ${summary}`,
    confirmLabel: t("Finish review and apply"),
  });
  if (!confirmed) return;

  state.maintenance.reviewCommitBusy = true;
  renderRelationReviews();
  const failures = [];
  for (const { review, decision } of staged) {
    state.maintenance.reviewBusy.add(review.candidateId);
    try {
      await maintenanceMutation(
        `/api/maintenance/reviews/${encodeURIComponent(review.candidateId)}/${decision.decision}`,
        {},
      );
      state.maintenance.reviewDecisions.delete(review.candidateId);
      state.maintenance.relationReviews = state.maintenance.relationReviews
        .filter((item) => item.candidateId !== review.candidateId);
    } catch (error) {
      decision.error = error.message || String(error);
      failures.push(decision.error);
    } finally {
      state.maintenance.reviewBusy.delete(review.candidateId);
      persistMaintenanceReviewSession();
      renderRelationReviews();
    }
  }
  state.maintenance.reviewCommitBusy = false;
  try {
    await Promise.all([loadRelationReviews(), loadOverview()]);
    renderMaintenanceStatus(await api("/api/maintenance"));
    if (failures.length) showError(new Error(t("Some review decisions could not be applied and remain reversible.")));
  } finally {
    renderRelationReviews();
  }
}

function maintenanceAvailable() {
  return Boolean(state.maintenance.status?.available);
}

const MAINTENANCE_PHASES = {
  pack: { label: "Pack", scanKey: "packing", operation: "pack", next: "summary", order: 1 },
  summary: { label: "Summary", scanKey: "summary", operation: "summary", next: "relation", order: 2 },
  relation: { label: "Relations", scanKey: "relation", operation: "relation", next: "topic", order: 3 },
  topic: { label: "Extract Topic Page", scanKey: "topic", operation: "topic", next: null, order: 4 },
};

const MAINTENANCE_STAGES = {
  scan: { label: "Scan candidates", order: 1 },
  analyze: { label: "Analyze suggestions", order: 2 },
  review: { label: "Review and apply", order: 3 },
};

const MAINTENANCE_PASSES = {
  pack: { label: "Pack maintenance", phases: ["pack"], order: 1 },
  semantic: { label: "Semantic maintenance", phases: ["summary", "relation"], order: 2 },
  topic: { label: "Extract Topic Page", phases: ["topic"], order: 3 },
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

function maintenancePassWorkCount(pass = maintenancePass()) {
  return maintenancePassPhases(pass)
    .reduce((total, phase) => total + maintenanceWorkCount(phase), 0);
}

function maintenanceWorkLabel(phase = maintenancePhase()) {
  return phase === "summary" ? t("Scanned eligible pages") : t("Scanned candidate groups");
}

function maintenanceEstimatedCalls(phase = maintenancePhase()) {
  return maintenanceScanForPhase(phase)?.estimatedModelCalls || 0;
}

function maintenanceCandidates() {
  const passPhases = new Set(maintenancePassPhases());
  return state.maintenance.pendingCandidates
    .filter((candidate) => passPhases.has(candidate.operation));
}

function candidateReviewState(candidate) {
  if (candidate.operation === "relation") return relationReviewState(candidate);
  return state.maintenance.candidateReviewStates.get(candidate.candidateId) || null;
}

function acceptMaintenanceCandidate(candidate) {
  if (candidate.operation === "relation") {
    acceptRelationCandidate(candidate);
    return;
  }
  state.maintenance.selected.add(candidate.candidateId);
  state.maintenance.candidateReviewStates.set(candidate.candidateId, "accepted");
  renderMaintenanceSession();
}

function skipMaintenanceCandidate(candidate) {
  if (candidate.operation === "relation") {
    skipRelationCandidate(candidate);
    return;
  }
  state.maintenance.selected.delete(candidate.candidateId);
  state.maintenance.candidateReviewStates.set(candidate.candidateId, "skipped");
  renderMaintenanceSession();
}

function undoMaintenanceCandidateDecision(candidate) {
  state.maintenance.selected.delete(candidate.candidateId);
  state.maintenance.candidateReviewStates.delete(candidate.candidateId);
  state.maintenance.relationReviewStates.delete(candidate.candidateId);
  state.maintenance.relationDraftStates.delete(candidate.candidateId);
  renderMaintenanceSession();
}

function maintenanceApplyStateNode(candidate) {
  const applyState = state.maintenance.applyStates.get(candidate.candidateId);
  if (!applyState) return null;
  const label = applyState.status === "running"
    ? (currentLanguage === "zh" ? "正在应用" : "Applying")
    : (currentLanguage === "zh" ? "应用失败，可重试" : "Apply failed; retry available");
  const node = element("span", `maintenance-apply-state ${applyState.status}`, label);
  if (applyState.message) node.title = applyState.message;
  return node;
}

function relationDraftState(candidate) {
  return state.maintenance.relationDraftStates.get(candidate.candidateId) || null;
}

function relationReviewState(candidate) {
  return state.maintenance.relationReviewStates.get(candidate.candidateId) || null;
}

function acceptRelationCandidate(candidate) {
  if (candidate.operation !== "relation" || relationDraftState(candidate) === "suppressed") return;
  stageRelationDecision(
    state.maintenance.selected,
    state.maintenance.relationReviewStates,
    candidate.candidateId,
    RELATION_DECISION.ACCEPT,
  );
  renderMaintenanceSession();
}

function rejectRelationCandidate(candidate) {
  if (candidate.operation !== "relation" || relationDraftState(candidate) === "suppressed") return;
  stageRelationDecision(
    state.maintenance.selected,
    state.maintenance.relationReviewStates,
    candidate.candidateId,
    RELATION_DECISION.REJECT,
  );
  renderMaintenanceSession();
}

function skipRelationCandidate(candidate) {
  if (candidate.operation !== "relation" || relationDraftState(candidate) === "suppressed") return;
  stageRelationDecision(
    state.maintenance.selected,
    state.maintenance.relationReviewStates,
    candidate.candidateId,
    RELATION_DECISION.SKIP,
  );
  renderMaintenanceSession();
}

function toggleRelationCandidateSuppression(candidate) {
  if (candidate.operation !== "relation") return;
  if (relationDraftState(candidate) === "suppressed") {
    state.maintenance.relationDraftStates.delete(candidate.candidateId);
  } else {
    state.maintenance.relationDraftStates.set(candidate.candidateId, "suppressed");
    state.maintenance.selected.delete(candidate.candidateId);
    state.maintenance.relationReviewStates.delete(candidate.candidateId);
  }
  renderMaintenanceSession();
}

function maintenanceApplySelection() {
  return partitionMaintenanceDecisions(
    maintenanceCandidates(),
    state.maintenance.selected,
    state.maintenance.relationReviewStates,
    state.maintenance.relationDraftStates,
  );
}

function maintenanceReviewedCandidates() {
  return maintenanceCandidates().filter((candidate) => (
    candidateReviewState(candidate) || relationDraftState(candidate) === "suppressed"
  ));
}

function maintenanceUnresolvedCandidates() {
  return maintenanceCandidates().filter((candidate) => (
    !candidateReviewState(candidate) && relationDraftState(candidate) !== "suppressed"
  ));
}

function maintenanceProposalKind(candidate) {
  if (candidate.operation === "summary") return t("Summary proposal");
  if (candidate.operation === "relation") return t("Relation proposal");
  if (candidate.operation === "topic") return t("Extract Topic Page");
  const mergesPacks = candidate.pages?.length === 2
    && candidate.pages.every((page) => page.mediaType === "application/vnd.pcp.packed-page+json");
  return t(mergesPacks ? "Merge Packs" : candidate.extendsExistingPack ? "Extend Pack" : "New Pack");
}

function maintenanceProposalSummary(candidate) {
  if (candidate.operation === "summary") return compactRelationReviewPreview(candidate.content, 180);
  if (candidate.operation === "relation") return compactRelationReviewPreview(candidate.relationReason, 180);
  if (candidate.operation === "topic") return compactRelationReviewPreview(candidate.title || candidate.content, 180);
  return `${formatNumber(candidate.inputPageCount)} ${t("Pages")} → ${formatNumber(candidate.resultingEntryCount)} ${t("entries")}`;
}

function maintenanceProposalSettledRow(candidate, decision) {
  const visualDecision = decision === "suppressed" ? "suppress" : decision === "skipped" ? "defer" : decision.replace(/ed$/, "");
  const row = element("article", `maintenance-review-settled-row tone-${visualDecision}`);
  const stateNode = element("span", "maintenance-review-settled-state");
  stateNode.append(
    visualDecision === "accept" ? acceptRelationIcon()
      : visualDecision === "defer" ? skipRelationIcon()
        : visualDecision === "suppress" ? suppressRelationIcon()
          : rejectRelationIcon(),
    element("strong", "", decision === "accepted"
      ? t("Accepted")
      : decision === "rejected"
        ? t("Rejected for this review")
        : decision === "suppressed"
          ? t("Will not be suggested when applied")
          : t("Skipped for now")),
  );
  const summary = element("span", "maintenance-review-settled-summary", maintenanceProposalSummary(candidate));
  summary.title = summary.textContent;
  const applyState = state.maintenance.applyStates.get(candidate.candidateId);
  const pending = element(
    "span",
    `maintenance-review-settled-pending${applyState?.status === "failed" ? " has-error" : ""}`,
    applyState?.status === "running"
      ? (currentLanguage === "zh" ? "正在应用" : "Applying")
      : applyState?.status === "failed"
        ? (currentLanguage === "zh" ? "应用失败，可撤销或重试" : "Apply failed; undo or retry")
        : t("Pending commit"),
  );
  if (applyState?.message) pending.title = applyState.message;
  const undo = element("button", "compact-button maintenance-review-undo", t("Undo"));
  undo.type = "button";
  undo.prepend(undoIcon());
  undo.addEventListener("click", () => undoMaintenanceCandidateDecision(candidate));
  row.append(
    element("span", `maintenance-review-kind maintenance-review-kind-${candidate.operation}`, maintenanceProposalKind(candidate)),
    stateNode,
    summary,
    pending,
    undo,
  );
  return row;
}

function maintenanceProposalActions(candidate, { allowReject = false } = {}) {
  const actions = element("div", "maintenance-proposal-actions");
  const accept = element("button", "compact-button maintenance-proposal-accept", t("Accept"));
  accept.type = "button";
  accept.prepend(acceptRelationIcon());
  accept.addEventListener("click", () => acceptMaintenanceCandidate(candidate));
  actions.append(accept);
  if (allowReject) {
    const reject = element("button", "compact-button maintenance-proposal-reject", t("Reject"));
    reject.type = "button";
    reject.prepend(rejectRelationIcon());
    reject.addEventListener("click", () => rejectRelationCandidate(candidate));
    actions.append(reject);
  }
  const skip = element("button", "compact-button maintenance-proposal-skip", t("Skip for now"));
  skip.type = "button";
  skip.prepend(skipRelationIcon());
  skip.addEventListener("click", () => skipMaintenanceCandidate(candidate));
  actions.append(skip);
  return actions;
}

function summaryProposalCard(candidate) {
  const reviewState = candidateReviewState(candidate);
  if (reviewState) return maintenanceProposalSettledRow(candidate, reviewState);
  const card = element("article", "maintenance-summary-card");
  const heading = element("div", "maintenance-summary-card-heading");
  const label = element("strong", "maintenance-proposal-kind", t("Summary proposal"));
  const source = element("div", "maintenance-summary-source");
  source.append(
    element("strong", "", candidate.namespace),
    element("span", "mono muted", candidate.pageId),
  );
  const open = element("button", "compact-button compact-icon-button");
  open.type = "button";
  open.title = t("Open page");
  open.setAttribute("aria-label", t("Open page"));
  open.append(openPageIcon());
  open.addEventListener("click", () => pageInspector.open(candidate.pageId));
  heading.append(label, source, open);
  const metadata = element("div", "maintenance-summary-metadata");
  metadata.textContent = `${t("Summary route")} · ${formatSize(candidate.contentChars)} ${t("Page")}`;
  const applyState = maintenanceApplyStateNode(candidate);
  if (applyState) metadata.append(" · ", applyState);
  card.append(heading, metadata, element("div", "maintenance-summary-content", candidate.content), maintenanceProposalActions(candidate));
  return card;
}

function relationProposalCard(candidate) {
  const draftState = relationDraftState(candidate);
  const reviewState = relationReviewState(candidate);
  if (draftState === "suppressed") return maintenanceProposalSettledRow(candidate, "suppressed");
  if (reviewState) return maintenanceProposalSettledRow(candidate, reviewState);
  const card = element("article", "maintenance-relation-proposal");
  const aside = element("div", "maintenance-relation-proposal-aside");
  aside.append(
    element("span", "maintenance-relation-proposal-kind", t("Relation proposal")),
    element("strong", "", candidate.namespace),
    element("span", "muted", `${candidate.pages.length} ${t("Pages")}`),
  );
  const applyState = maintenanceApplyStateNode(candidate);
  if (applyState) aside.append(applyState);

  const body = element("div", "maintenance-relation-proposal-body");
  const pages = element("div", "maintenance-relation-proposal-pages");
  candidate.pages.forEach((page, index) => {
    const item = element("div", "maintenance-relation-proposal-page");
    const heading = element("div", "maintenance-relation-proposal-page-heading");
    heading.append(
      element("strong", "", `${t("Page")} ${index + 1}`),
      element("span", "mono muted", page.pageId),
    );
    item.append(heading, element("div", "maintenance-relation-proposal-preview", compactRelationReviewPreview(page.preview || t("No preview"), 300)));
    pages.append(item);
  });
  const rationale = element("div", "maintenance-relation-proposal-reason");
  rationale.append(
    element("span", "maintenance-relation-evidence-label", t("Relation evidence")),
    element("span", "", candidate.relationReason || t("No relation rationale was supplied.")),
  );
  const actions = element("div", "maintenance-relation-proposal-actions");
  const view = relationComparisonButton(candidate, "icon-button maintenance-relation-action-button maintenance-relation-view", {
    iconOnly: true,
    onAccept: () => acceptRelationCandidate(candidate),
    onReject: () => rejectRelationCandidate(candidate),
    onSkip: () => skipRelationCandidate(candidate),
  });
  view.title = t("View relation Pages");
  view.setAttribute("aria-label", view.title);
  view.replaceChildren(openPageIcon());
  const suppress = element("button", "maintenance-relation-no-suggest", t("Do not suggest this relation again"));
  suppress.type = "button";
  suppress.addEventListener("click", () => toggleRelationCandidateSuppression(candidate));
  actions.append(suppress, view, maintenanceProposalActions(candidate, { allowReject: true }));
  body.append(pages, rationale, actions);
  card.append(aside, body);
  return card;
}

function genericProposalCard(candidate) {
  const reviewState = candidateReviewState(candidate);
  if (reviewState) return maintenanceProposalSettledRow(candidate, reviewState);
  const card = element("article", "maintenance-generic-card");
  const header = element("div", "maintenance-generic-card-heading");
  const title = element("div", "maintenance-generic-card-title");
  title.append(
    element("span", "maintenance-proposal-kind", maintenanceProposalKind(candidate)),
    element("strong", "", candidate.operation === "topic" ? candidate.title : candidate.namespace),
  );
  const meta = candidate.operation === "topic"
    ? `${candidate.pages.length} ${t("Pages")}`
    : `${t("Stream")} ${candidate.sourceSpan.start}–${candidate.sourceSpan.end} · ${formatSize(candidate.contentChars)}`;
  header.append(title, element("span", "muted", meta));
  const body = element("div", "maintenance-generic-card-body");
  if (candidate.operation === "topic") {
    if (candidate.reason) {
      const rationale = element("div", "maintenance-relation-proposal-reason");
      rationale.append(
        element("span", "maintenance-relation-evidence-label", t("Why this Topic Page is worth creating")),
        element("span", "", candidate.reason),
      );
      body.append(rationale);
    }
    body.append(element("div", "maintenance-generic-preview", compactRelationReviewPreview(candidate.content, 360)));
    body.append(topicExtractionReviewButton(candidate));
  } else {
    const pages = element("div", "maintenance-generic-pages");
    for (const page of candidate.pages) {
      const item = element("div", "maintenance-generic-page");
      item.append(
        element("span", "mono muted", `${page.sourceSpan.start}–${page.sourceSpan.end}`),
        element("span", "maintenance-preview", compactRelationReviewPreview(page.preview || page.pageId, 220)),
      );
      pages.append(item);
    }
    body.append(
      element("strong", "maintenance-generic-outcome", maintenanceProposalSummary(candidate)),
      pages,
    );
  }
  const applyState = maintenanceApplyStateNode(candidate);
  if (applyState) body.append(applyState);
  card.append(header, body, maintenanceProposalActions(candidate));
  return card;
}

function renderMaintenanceProposals(candidates) {
  const summaryCandidates = candidates.filter((candidate) => candidate.operation === "summary");
  const relationCandidates = candidates.filter((candidate) => candidate.operation === "relation");
  const genericCandidates = candidates.filter((candidate) => candidate.operation !== "summary" && candidate.operation !== "relation");
  const summaryCards = byId("maintenance-summary-cards");
  const relationCards = byId("maintenance-relation-cards");
  const genericCards = byId("maintenance-generic-cards");
  summaryCards.hidden = summaryCandidates.length === 0;
  relationCards.hidden = relationCandidates.length === 0;
  genericCards.hidden = genericCandidates.length === 0;
  if (summaryCandidates.length) summaryCards.replaceChildren(...summaryCandidates.map(summaryProposalCard));
  if (relationCandidates.length) relationCards.replaceChildren(...relationCandidates.map(relationProposalCard));
  if (genericCandidates.length) genericCards.replaceChildren(...genericCandidates.map(genericProposalCard));
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
  if (maintenanceWorkflowStage() === "review" && maintenanceFailedBatches().length) return t("Analysis incomplete");
  const { candidates, rejections, suppressions } = maintenanceApplySelection();
  return candidates.length + rejections.length + suppressions.length ? t("Ready to apply") : t("Ready to continue");
}

function maintenancePhaseDescription() {
  const phase = maintenancePhase();
  const scan = maintenanceScanForPhase();
  const analysis = state.maintenance.analyses[phase];
  if (!scan) return currentLanguage === "zh"
    ? "正在等待扫描完整的可处理库存。"
    : "Waiting to scan the full eligible inventory.";
  if (!analysis && maintenanceWorkCount() === 0) {
    const next = maintenancePassConfig().order < Object.keys(MAINTENANCE_PASSES).length;
    return currentLanguage === "zh"
      ? (next
        ? "没有发现候选；无需调用模型，可直接继续下一维护段。"
        : "没有发现候选；无需调用模型，可直接完成维护。")
      : (next
        ? "No candidates found. No model call is needed; continue directly to the next pass."
        : "No candidates found. No model call is needed; complete maintenance directly.");
  }
  if (!analysis) return currentLanguage === "zh"
      ? `扫描已发现 ${formatNumber(maintenanceWorkCount())} 个结构候选。它们不是已建议的变更；点击分析后才会调用模型判断是否应合并、摘要、凝练新页或关联。`
      : `The scan found ${formatNumber(maintenanceWorkCount())} structural candidates. They are not recommendations yet: analysis calls a model to decide whether to pack, summarize, extract a Topic Page, or relate them.`;
  const failedBatches = maintenanceFailedBatches();
  if (failedBatches.length) return currentLanguage === "zh"
    ? `${formatNumber(failedBatches.length)} 个分析批次未完成。其他批次的提案仍然保留；可单独重试失败批次，或继续审阅已有结果。`
    : `${formatNumber(failedBatches.length)} analysis batch${failedBatches.length === 1 ? " is" : "es are"} incomplete. Proposals from other batches remain available; retry only the failed batches or continue reviewing the available results.`;
  if (maintenanceCandidates().length) return currentLanguage === "zh"
    ? "逐项接受、拒绝或跳过。决定会先在本次会话内暂存，应用前仍可撤销。"
    : "Accept, reject, or skip each proposal. Decisions are staged in this session and remain undoable until applying.";
  return t("Analysis completed. No changes are recommended for this stage. Continue when you are ready.");
}

function currentMaintenanceAction() {
  if (!maintenanceSessionActive()) return "start";
  const stage = maintenanceWorkflowStage();
  if (stage === "scan") return "scan";
  if (stage === "analyze") return "analyze";
  const { candidates, rejections, suppressions } = maintenanceApplySelection();
  return candidates.length + rejections.length + suppressions.length ? "apply" : "advance";
}

function maintenancePrimaryLabel(action = currentMaintenanceAction()) {
  if (action === "start") return t("Start maintenance");
  if (action === "scan") return t("Scan candidates");
  if (action === "analyze") return t("Analyze suggestions");
  if (action === "apply") {
    const { candidates, rejections, suppressions } = maintenanceApplySelection();
    const count = candidates.length + rejections.length + suppressions.length;
    return `${t("Apply decisions")} (${formatNumber(count)})`;
  }
  if (maintenancePass() === "pack") return t("Continue to semantic maintenance");
  if (maintenancePass() === "semantic") return t("Continue to Topic Page extraction");
  return t("Complete maintenance");
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
}

function renderMaintenancePasses() {
  const currentPass = maintenancePass();
  const currentConfig = maintenancePassConfig();
  const sessionComplete = maintenanceSessionComplete();
  const active = maintenanceSessionActive();

  byId("maintenance-flow-kicker").textContent = currentLanguage === "zh"
    ? "维护会话 · 三段流程"
    : "Maintenance session · three-stage flow";
  byId("maintenance-flow-title").textContent = currentLanguage === "zh"
    ? "先确定边界，再建立语义，最后凝练主题页"
    : "Set boundaries, establish semantics, then extract Topic Pages";
  byId("maintenance-flow-description").textContent = currentLanguage === "zh"
    ? "每一段都会在上一段应用或跳过后重新扫描；凝练新页因此使用已经确认的关系结构。"
    : "Each stage rescans after the prior stage is applied or skipped, so Topic Pages use the confirmed relation structure.";

  for (const [pass, config] of Object.entries(MAINTENANCE_PASSES)) {
    const item = byId(`maintenance-pass-${pass}`);
    const title = byId(`maintenance-pass-${pass}-title`);
    const detail = byId(`maintenance-pass-${pass}-detail`);
    const status = byId(`maintenance-pass-${pass}-status`);
    const completed = sessionComplete || (active && config.order < currentConfig.order);
    const isActive = active && pass === currentPass;
    const waiting = !completed && !isActive;
    item.classList.toggle("active", isActive);
    item.classList.toggle("completed", completed);
    item.classList.toggle("waiting", waiting);
    title.textContent = t(config.label);
    detail.textContent = currentLanguage === "zh"
      ? (pass === "pack"
        ? "Pack 边界与合并"
        : pass === "semantic"
          ? "摘要与关系建议"
          : "基于已确认关系凝练主题页")
      : (pass === "pack"
        ? "Pack boundaries and merges"
        : pass === "semantic"
          ? "Summary and relation proposals"
          : "Extract from confirmed relations");
    status.textContent = completed
      ? t("Completed")
      : isActive && state.maintenance.busy
        ? (currentLanguage === "zh" ? "处理中" : "Working")
        : isActive
          ? (currentLanguage === "zh" ? "当前阶段" : "Current pass")
          : t("Waiting");
  }

  byId("maintenance-stage-track-kicker").textContent = currentLanguage === "zh"
    ? `第 ${currentConfig.order} 段 · 当前工作流`
    : `Pass ${currentConfig.order} · current workflow`;
  byId("maintenance-stage-track-title").textContent = t(currentConfig.label);
  byId("maintenance-stage-track-status").textContent = sessionComplete
    ? t("Completed")
    : state.maintenance.busy
      ? (currentLanguage === "zh" ? "处理中" : "Working")
      : `${maintenanceStageConfig().order} / 3`;
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
  const reviewedCount = maintenanceReviewedCandidates().length;
  const unresolvedCount = Math.max(0, candidates.length - reviewedCount);

  renderMaintenancePasses();
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
    : stage === "analyze" && pass === "semantic"
      ? (currentLanguage === "zh"
        ? "模型逐批评估摘要与关联候选；提案一旦返回即可先审阅，但全部批次结束前不能应用，也不会写入页面。"
        : "The model evaluates summary and relation candidates batch by batch. Returned proposals can be reviewed immediately, but applying stays locked and no Page is written until every batch finishes.")
    : stage === "analyze"
      ? (currentLanguage === "zh"
        ? "模型逐批评估提案；已返回的内容可以先审阅，但全部批次结束前不能应用。"
        : "The model evaluates proposals batch by batch. Returned proposals can be reviewed immediately, but applying stays locked until every batch finishes.")
      : stage === "review" && candidates.length === 0
        ? (currentLanguage === "zh"
          ? (issues.length
            ? "分析已结束，但没有形成可应用提案；失败批次可单独重试，其余批次没有建议变更。"
            : "分析已结束，没有建议变更；可直接继续下一维护段。")
          : (issues.length
            ? "Analysis finished without an applicable proposal. Failed batches can be retried independently; the remaining batches recommended no change."
            : "Analysis finished with no recommended change; continue directly to the next pass."))
      : (currentLanguage === "zh"
        ? "统一审阅所有建议。每个决定先暂存在本次会话；应用前可撤销，写入时会重新校验当前版本。"
        : "Review every suggestion together. Each decision is staged for this session, remains undoable before applying, and revalidates the current revision when written.");

  const metrics = stage === "scan"
    ? [
      metric(t("Scanned candidate groups"), formatNumber(scanGroups)),
      metric(t("Scanned eligible pages"), formatNumber(scanPages)),
      metric(t("Estimated calls"), formatNumber(estimatedCalls)),
    ]
    : [
      metric(t("Scanned candidate groups"), formatNumber(scanGroups)),
      metric(t("Scanned eligible pages"), formatNumber(scanPages)),
      metric(
        t("Model calls"),
        formatNumber(modelCalls),
        modelCalls ? "info" : "",
        currentLanguage === "zh"
          ? "包含成功、失败与重试的调用尝试；各批次结果见下方日志。"
          : "Includes successful, failed, and retried attempts; see the batch log below.",
      ),
      metric(t("Proposals"), formatNumber(candidates.length), candidates.length ? "positive" : ""),
    ];
  const metricGrid = byId("maintenance-scan-metrics");
  metricGrid.classList.toggle("four-columns", metrics.length === 4);
  metricGrid.replaceChildren(...metrics);
  byId("maintenance-candidate-status").textContent = stage === "review"
    ? `${formatNumber(candidates.length)} ${t("proposals")} · ${formatNumber(reviewedCount)} ${currentLanguage === "zh" ? "已审" : "reviewed"} · ${formatNumber(unresolvedCount)} ${currentLanguage === "zh" ? "待定" : "remaining"}`
    : stage === "analyze"
      ? (currentLanguage === "zh"
        ? `已处理 ${formatNumber(maintenanceProcessedCalls())} / ${formatNumber(estimatedCalls)} 批 · ${formatNumber(candidates.length)} 个提案可审阅`
        : `${formatNumber(maintenanceProcessedCalls())} of ${formatNumber(estimatedCalls)} batches processed · ${formatNumber(candidates.length)} proposals available to review`)
      : `${formatNumber(scanGroups + scanPages)} ${currentLanguage === "zh" ? "扫描项" : "scanned items"}`;

  const failedBatches = maintenanceFailedBatches();
  const issueDetails = issues.map((item) => item.message || String(item)).join("\n");
  const failureProgress = failedBatches.length
    ? (currentLanguage === "zh"
      ? `${formatNumber(failedBatches.length)} 个批次失败；成功批次的提案已保留${state.maintenance.busy ? "，其余批次仍在继续。" : "。"}`
      : `${formatNumber(failedBatches.length)} batches failed. Proposals from successful batches were retained${state.maintenance.busy ? " while remaining batches continue." : "."}`)
    : "";
  const issue = [failureProgress, issueDetails].filter(Boolean).join("\n");
  const issueNode = byId("maintenance-issue");
  issueNode.hidden = !issue;
  issueNode.textContent = issue ? `${t("Analysis incomplete")}: ${issue}` : "";

  const analysisLog = byId("maintenance-analysis-log");
  const analysisLogSummary = byId("maintenance-analysis-log-summary");
  const analysisLogBody = byId("maintenance-analysis-log-body");
  const logLines = [];
  for (const phase of maintenancePassPhases()) {
    const analysis = state.maintenance.analyses[phase];
    if (!analysis?.batches?.length) continue;
    const progress = batchProgress(analysis.batches);
    const phaseLabel = t(maintenancePhaseConfig(phase).label);
    logLines.push(currentLanguage === "zh"
      ? `${phaseLabel}：${formatNumber(progress.completed)} 个完成，${formatNumber(progress.failed)} 个失败，${formatNumber(progress.running)} 个处理中，共 ${formatNumber(progress.total)} 个。`
      : `${phaseLabel}: ${formatNumber(progress.completed)} complete, ${formatNumber(progress.failed)} failed, ${formatNumber(progress.running)} running, ${formatNumber(progress.total)} total.`);
    const noteworthy = analysis.batches.filter((batch) => batch.status !== "completed" || batch.issue);
    const visible = noteworthy.length
      ? noteworthy.slice(0, 16)
      : analysis.batches.slice(Math.max(0, analysis.batches.length - 5));
    const itemLabel = phase === "summary"
      ? (currentLanguage === "zh" ? "页面" : "Page")
      : (currentLanguage === "zh" ? "批次" : "Batch");
    for (const batch of visible) {
      const stateLabel = currentLanguage === "zh"
        ? ({ pending: "等待", running: "处理中", completed: "完成", failed: "失败" }[batch.status] || batch.status)
        : batch.status;
      const decisionLabel = currentLanguage === "zh"
        ? ({ candidate: "形成提案", defer: "延后", no_candidate: "无提案", none: "无提案" }[batch.decision] || batch.decision)
        : batch.decision;
      const label = currentLanguage === "zh"
        ? `第 ${batch.batchIndex + 1} ${itemLabel} · ${stateLabel}${decisionLabel ? ` · ${decisionLabel}` : ""}${batch.issue ? ` · ${batch.issue}` : ""}`
        : `${itemLabel} ${batch.batchIndex + 1} · ${batch.status}${batch.decision ? ` · ${batch.decision}` : ""}${batch.issue ? ` · ${batch.issue}` : ""}`;
      logLines.push(label);
    }
    if (noteworthy.length > visible.length) {
      logLines.push(currentLanguage === "zh"
        ? `另有 ${formatNumber(noteworthy.length - visible.length)} 个待处理或失败批次未展开。`
        : `${formatNumber(noteworthy.length - visible.length)} additional pending or failed batches are not expanded.`);
    }
  }
  analysisLog.hidden = logLines.length === 0;
  analysisLogSummary.textContent = logLines.length ? logLines[0] : "";
  analysisLogBody.textContent = logLines.slice(1).join("\n");

  const proposals = byId("maintenance-proposals");
  proposals.hidden = !["analyze", "review"].includes(stage) || candidates.length === 0;
  if (!proposals.hidden) {
    proposals.classList.toggle("is-live", stage === "analyze");
    renderMaintenanceProposals(candidates);
    byId("maintenance-selection-status").textContent = stage === "analyze"
      ? (currentLanguage === "zh"
        ? `分析仍在继续 · 已审 ${formatNumber(reviewedCount)} / ${formatNumber(candidates.length)} · 完成前不能应用`
        : `Analysis is still running · ${formatNumber(reviewedCount)} of ${formatNumber(candidates.length)} reviewed · applying is locked`)
      : (currentLanguage === "zh"
        ? `已审 ${formatNumber(reviewedCount)} / ${formatNumber(candidates.length)} · ${formatNumber(unresolvedCount)} 待定`
        : `${formatNumber(reviewedCount)} of ${formatNumber(candidates.length)} reviewed · ${formatNumber(unresolvedCount)} remaining`);
  }

  const primary = byId("maintenance-primary");
  const action = currentMaintenanceAction();
  updateMaintenanceButton(primary, maintenancePrimaryLabel(action), state.maintenance.busy, state.maintenance.activity);
  primary.disabled = !maintenanceAvailable() || state.maintenance.busy;
  const rescan = byId("maintenance-rescan");
  rescan.disabled = state.maintenance.busy;
  const skip = byId("maintenance-skip");
  skip.hidden = stage !== "review" || candidates.length === 0;
  skip.disabled = state.maintenance.busy;
  const retryFailed = byId("maintenance-retry-failed");
  const failedBatchCount = maintenanceFailedBatches().length;
  retryFailed.hidden = stage !== "review" || failedBatchCount === 0;
  retryFailed.disabled = state.maintenance.busy || failedBatchCount === 0;
  retryFailed.textContent = `${t("Retry failed batches")} (${formatNumber(failedBatchCount)})`;
  const cancel = byId("maintenance-cancel");
  cancel.disabled = state.maintenance.busy;
  if (!state.maintenance.busy) {
    byId("maintenance-status").textContent = `${t(stageConfig.label)} · ${stage === "review" && candidates.length ? t("Ready to apply") : t("Ready to continue")}`;
  }
}

function renderMaintenanceReport() {
  renderMaintenancePasses();
  const outcomes = state.maintenance.session.outcomes;
  const totalCalls = Object.values(outcomes).reduce((sum, outcome) => sum + outcome.modelCalls, 0);
  const totalSkipped = Object.values(outcomes).reduce((sum, outcome) => sum + outcome.skipped, 0);
  byId("maintenance-report-status").textContent = `${t("Maintenance session completed")} · ${formatTime(state.maintenance.session.completedAt)}`;
  byId("maintenance-report-metrics").replaceChildren(
    metric(t("Pack"), `${formatNumber(outcomes.pack.applied)} ${t("Applied")}`, outcomes.pack.applied ? "positive" : ""),
    metric(t("Summary"), `${formatNumber(outcomes.summary.applied)} ${t("Applied")}`, outcomes.summary.applied ? "positive" : ""),
    metric(
      t("Relations"),
      currentLanguage === "zh"
        ? `${formatNumber(outcomes.relation.applied)} 接受 · ${formatNumber(outcomes.relation.rejected)} 拒绝 · ${formatNumber(outcomes.relation.suppressed)} 不再建议`
        : `${formatNumber(outcomes.relation.applied)} accepted · ${formatNumber(outcomes.relation.rejected)} rejected · ${formatNumber(outcomes.relation.suppressed)} suppressed`,
      outcomes.relation.applied || outcomes.relation.rejected || outcomes.relation.suppressed ? "positive" : "",
    ),
    metric(t("Extract Topic Page"), `${formatNumber(outcomes.topic.applied)} ${t("Applied")}`, outcomes.topic.applied ? "positive" : ""),
    metric(t("Model calls"), formatNumber(totalCalls), totalCalls ? "info" : ""),
    metric(t("Skipped"), formatNumber(totalSkipped), totalSkipped ? "warning" : ""),
  );
  byId("maintenance-status").textContent = t("Maintenance session completed");
}

function renderMaintenanceSession() {
  renderMaintenanceConvergenceState();
  const idle = !maintenanceSessionActive() && !maintenanceSessionComplete();
  byId("maintenance-idle").hidden = !idle;
  byId("maintenance-workflow").hidden = !maintenanceSessionActive();
  byId("maintenance-report").hidden = !maintenanceSessionComplete();
  byId("maintenance-start").disabled = !maintenanceAvailable() || state.maintenance.busy || archiveSessionActive();
  byId("maintenance-manual-start").disabled = !maintenanceAvailable() || state.maintenance.busy || archiveSessionActive();
  if (idle) {
    const convergence = state.maintenance.convergence;
    byId("maintenance-status").textContent = convergence.running
      ? state.maintenance.activity || t("Working")
      : maintenanceSceneError()
        ? t("Needs attention")
      : convergence.completedAt
        ? `${t(state.maintenance.relationReviews.length ? "Awaiting review" : "Maintenance converged")} · ${formatTime(convergence.completedAt)}`
        : maintenanceAvailable()
          ? t("Awaiting run")
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
    batches: Array.from({ length: scan.estimatedModelCalls }, (_, batchIndex) => ({
      batchIndex,
      status: "pending",
      attempts: 0,
      workerCalls: 0,
      analyzedGroupCount: 0,
      overlapRetries: 0,
      noCandidateGroups: 0,
      deferredGroups: 0,
      candidateIds: [],
      candidates: [],
      issues: [],
      issue: null,
    })),
  };
}

function refreshMaintenanceAnalysisTotals(analysis) {
  const progress = batchProgress(analysis.batches);
  analysis.batchesCompleted = progress.processed;
  analysis.workerCalls = analysis.batches.reduce((total, batch) => total + (batch.workerCalls || 0), 0);
  analysis.analyzedGroupCount = analysis.batches.reduce((total, batch) => total + (batch.analyzedGroupCount || 0), 0);
  analysis.overlapRetries = analysis.batches.reduce((total, batch) => total + (batch.overlapRetries || 0), 0);
  analysis.noCandidateGroups = analysis.batches.reduce((total, batch) => total + (batch.noCandidateGroups || 0), 0);
  analysis.deferredGroups = analysis.batches.reduce((total, batch) => total + (batch.deferredGroups || 0), 0);
  analysis.candidates = analysis.batches.flatMap((batch) => batch.candidates || []);
  analysis.issues = analysis.batches.flatMap((batch) => batch.issues || []);
}

async function loadMaintenance({ reload = false } = {}) {
  if (!state.maintenance.loaded || reload) renderMaintenanceStatus(await api("/api/maintenance"));
  else renderMaintenanceSession();
  renderArchiveSession();
  await loadRelationReviews();
  scheduleMaintenanceStatusPoll();
}

function scheduleMaintenanceStatusPoll() {
  if (maintenanceStatusPoll != null) window.clearTimeout(maintenanceStatusPoll);
  maintenanceStatusPoll = null;
  if (state.activeView !== "maintenance") return;
  const delay = state.maintenance.status?.automation?.state === "running" ? 2_000 : 15_000;
  maintenanceStatusPoll = window.setTimeout(async () => {
    try {
      state.maintenance.status = await api("/api/maintenance");
      state.maintenance.loaded = true;
      renderAutomationStatus();
    } catch (_) {
      // A status poll is advisory; the next scheduled read can recover without
      // replacing the operator's current review state with an error screen.
    } finally {
      scheduleMaintenanceStatusPoll();
    }
  }, delay);
}

async function requestMaintenanceScan() {
  return maintenanceMutation("/api/maintenance/scan", {});
}

async function applyMaintenanceCandidates(candidates, onProgress) {
  let applied = 0;
  const appliedCandidateIds = [];
  const skipped = [];
  for (const [index, candidate] of candidates.entries()) {
    onProgress?.({ index, total: candidates.length, applied, skipped, candidate, status: "running" });
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
      } else if (candidate.operation === "topic") {
        await maintenanceMutation("/api/maintenance/topics/apply", {
          candidateId: candidate.candidateId,
          title: candidate.title,
          content: candidate.content,
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
      onProgress?.({ index: index + 1, total: candidates.length, applied, skipped, candidate, status: "applied" });
    } catch (error) {
      const failure = { candidateId: candidate.candidateId, message: error.message || String(error) };
      skipped.push(failure);
      onProgress?.({ index: index + 1, total: candidates.length, applied, skipped, candidate, status: "failed", error: failure.message });
    }
  }
  return { applied, appliedCandidateIds, skipped };
}

async function applyRelationRejections(candidates, onProgress) {
  let applied = 0;
  const appliedCandidateIds = [];
  const skipped = [];
  for (const [index, candidate] of candidates.entries()) {
    onProgress?.({ index, total: candidates.length, applied, skipped, candidate, status: "running" });
    try {
      await maintenanceMutation("/api/maintenance/relations/reject", {
        candidateId: candidate.candidateId,
        pages: candidate.pages.map((page) => ({ pageId: page.pageId, revisionId: page.revisionId })),
      });
      applied += 1;
      appliedCandidateIds.push(candidate.candidateId);
      onProgress?.({ index: index + 1, total: candidates.length, applied, skipped, candidate, status: "applied" });
    } catch (error) {
      const failure = { candidateId: candidate.candidateId, message: error.message || String(error) };
      skipped.push(failure);
      onProgress?.({ index: index + 1, total: candidates.length, applied, skipped, candidate, status: "failed", error: failure.message });
    }
  }
  return { applied, appliedCandidateIds, skipped };
}

async function applyRelationSuppressions(candidates, onProgress) {
  let applied = 0;
  const appliedCandidateIds = [];
  const skipped = [];
  for (const [index, candidate] of candidates.entries()) {
    onProgress?.({ index, total: candidates.length, applied, skipped, candidate, status: "running" });
    try {
      await maintenanceMutation("/api/maintenance/relations/suppress", {
        candidateId: candidate.candidateId,
        pages: candidate.pages.map((page) => ({ pageId: page.pageId, revisionId: page.revisionId })),
      });
      applied += 1;
      appliedCandidateIds.push(candidate.candidateId);
      onProgress?.({ index: index + 1, total: candidates.length, applied, skipped, candidate, status: "applied" });
    } catch (error) {
      const failure = { candidateId: candidate.candidateId, message: error.message || String(error) };
      skipped.push(failure);
      onProgress?.({ index: index + 1, total: candidates.length, applied, skipped, candidate, status: "failed", error: failure.message });
    }
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
  const candidateIds = [];
  for (const candidate of candidates || []) {
    const proposal = { ...candidate, operation };
    const existing = state.maintenance.pendingCandidates.findIndex((item) => item.candidateId === proposal.candidateId);
    if (existing >= 0) state.maintenance.pendingCandidates.splice(existing, 1, proposal);
    else state.maintenance.pendingCandidates.push(proposal);
    candidateIds.push(proposal.candidateId);
  }
  return candidateIds;
}

function replaceMaintenanceBatchCandidates(operation, batch, candidates) {
  const previousCandidateIds = new Set(batch.candidateIds || []);
  const candidateIds = appendMaintenanceCandidates(operation, candidates);
  const nextCandidateIds = new Set(candidateIds);
  for (const candidateId of previousCandidateIds) {
    if (nextCandidateIds.has(candidateId)) continue;
    state.maintenance.pendingCandidates = state.maintenance.pendingCandidates
      .filter((candidate) => candidate.candidateId !== candidateId);
    state.maintenance.selected.delete(candidateId);
    state.maintenance.relationDraftStates.delete(candidateId);
    state.maintenance.relationReviewStates.delete(candidateId);
    state.maintenance.candidateReviewStates.delete(candidateId);
  }
  batch.candidateIds = candidateIds;
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
      candidateIds: [],
      issues: [],
    })),
  };
}

function summaryFailedBatches() {
  return (state.maintenance.analyses.summary?.batches || [])
    .filter((batch) => batch.status === "failed");
}

function packFailedBatches() {
  return (state.maintenance.analyses.pack?.batches || [])
    .filter((batch) => batch.status === "failed");
}

function relationFailedBatches() {
  return (state.maintenance.analyses.relation?.batches || [])
    .filter((batch) => batch.status === "failed");
}

function topicFailedBatches() {
  return (state.maintenance.analyses.topic?.batches || [])
    .filter((batch) => batch.status === "failed");
}

function maintenanceFailedBatches() {
  return [
    ...packFailedBatches(),
    ...summaryFailedBatches(),
    ...relationFailedBatches(),
    ...topicFailedBatches(),
  ];
}

function maintenanceProcessedCalls() {
  return maintenancePassPhases().reduce((total, phase) => {
    const analysis = state.maintenance.analyses[phase];
    if (!analysis) return total;
    if (Array.isArray(analysis.batches)) {
      return total + analysis.batches.filter((batch) => (
        batch.status === "completed" || batch.status === "failed"
      )).length;
    }
    return total + (analysis.batchesCompleted || 0);
  }, 0);
}

function syncMaintenanceAnalyzeProgress() {
  if (state.maintenance.activity?.kind !== "analyze" || state.maintenance.activity.retry) return;
  state.maintenance.activity.current = maintenanceProcessedCalls();
}

function refreshSummaryAnalysisTotals(analysis) {
  analysis.batchesCompleted = batchProgress(analysis.batches).processed;
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
    batches: scan.groups.map((group, batchIndex) => ({
      batchIndex,
      groupId: group.groupId,
      status: "pending",
      attempts: 0,
      workerCalls: 0,
      decision: null,
      candidateIds: [],
      issue: null,
    })),
  };
}

function emptyTopicAnalysis(scan) {
  return {
    analyzedAt: null,
    scanId: scan.scanId,
    batchCount: scan.estimatedModelCalls,
    batchesCompleted: 0,
    workerCalls: 0,
    noCandidateGroups: 0,
    deferredGroups: 0,
    issues: [],
    batches: scan.groups.map((group, batchIndex) => ({
      batchIndex,
      groupId: group.groupId,
      status: "pending",
      attempts: 0,
      workerCalls: 0,
      decision: null,
      candidateIds: [],
      issue: null,
    })),
  };
}

function refreshGroupAnalysisTotals(analysis) {
  analysis.batchesCompleted = batchProgress(analysis.batches).processed;
  analysis.workerCalls = analysis.batches.reduce((total, batch) => total + (batch.workerCalls || 0), 0);
  analysis.noCandidateGroups = analysis.batches.filter((batch) => (
    batch.status === "completed" && ["none", "no_candidate"].includes(batch.decision)
  )).length;
  analysis.deferredGroups = analysis.batches.filter((batch) => (
    batch.status === "failed" || batch.decision === "defer"
  )).length;
  analysis.issues = analysis.batches
    .filter((batch) => batch.issue)
    .map((batch) => ({ batchIndex: batch.batchIndex, groupId: batch.groupId, message: batch.issue }));
}

function resetCurrentMaintenanceWork() {
  state.maintenance.scan = null;
  state.maintenance.analyses = { pack: null, summary: null, topic: null, relation: null };
  state.maintenance.pendingCandidates = [];
  state.maintenance.selected.clear();
  state.maintenance.relationDraftStates.clear();
  state.maintenance.relationReviewStates.clear();
  state.maintenance.candidateReviewStates.clear();
  state.maintenance.applyStates.clear();
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
    state.maintenance.workflowStage = maintenancePassWorkCount() ? "analyze" : "review";
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

async function analyzePackingPhase(scan, { retryFailed = false } = {}) {
  let analysis = state.maintenance.analyses.pack;
  if (!retryFailed || !analysis || analysis.scanId !== scan.scanId) {
    analysis = emptyMaintenanceAnalysis(scan);
    state.maintenance.analyses.pack = analysis;
  }
  // Mechanical Pack merges are valid proposals with zero model calls. Keep
  // them in the same review queue as model-selected Pack candidates so a zero
  // worker count can never look like this pass was skipped.
  const indexes = runnableBatchIndexes(analysis.batches, { retryFailed });
  const retryStart = retryFailed ? state.maintenance.activity.current : 0;
  for (const [progressIndex, batchIndex] of indexes.entries()) {
    const batchState = analysis.batches[batchIndex];
    if (!batchState) continue;
    beginBatch(batchState);
    batchState.issues = [];
    batchState.candidates = [];
    state.maintenance.activity.current = retryFailed ? retryStart + progressIndex : maintenanceProcessedCalls();
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Pack")} ${formatNumber(batchIndex + 1)} / ${formatNumber(analysis.batchCount)}`;
    renderMaintenanceSession();
    try {
      const result = await maintenanceMutation("/api/maintenance/analyze", {
        scanId: scan.scanId,
        batchIndex,
      });
      replaceMaintenanceBatchCandidates("pack", batchState, result.candidates || []);
      analysis.analyzedAt = result.analyzedAt;
      batchState.workerCalls += result.workerCalls || 0;
      batchState.analyzedGroupCount = result.analyzedGroupCount || 0;
      batchState.overlapRetries = result.overlapRetries || 0;
      batchState.noCandidateGroups = result.noCandidateGroups || 0;
      batchState.deferredGroups = result.deferredGroups || 0;
      batchState.candidates = result.candidates || [];
      batchState.issues = result.issue
        ? [{ ...result.issue, batchIndex, message: result.issue.message || String(result.issue) }]
        : [];
      if (batchState.issues.length) failBatch(batchState, batchState.issues[0].message);
      else completeBatch(batchState);
    } catch (error) {
      batchState.workerCalls += 1;
      batchState.deferredGroups = 1;
      batchState.issues = [{ batchIndex, message: error.message || String(error) }];
      failBatch(batchState, error);
    }
    refreshMaintenanceAnalysisTotals(analysis);
    if (retryFailed) state.maintenance.activity.current = retryStart + progressIndex + 1;
    else syncMaintenanceAnalyzeProgress();
    renderMaintenanceSession();
  }
}

async function analyzeSummaryPhase(scan, { retryFailed = false } = {}) {
  let analysis = state.maintenance.analyses.summary;
  if (!retryFailed || !analysis || analysis.scanId !== scan.scanId) {
    analysis = emptySummaryAnalysis(scan);
    state.maintenance.analyses.summary = analysis;
  }
  const batches = maintenanceBatches(scan.pages);
  const batchIndexes = runnableBatchIndexes(analysis.batches, { retryFailed });
  const retryStart = retryFailed ? state.maintenance.activity.current : 0;
  for (const [progressIndex, index] of batchIndexes.entries()) {
    const batch = batches[index];
    const batchState = analysis.batches[index];
    if (!batch || !batchState) continue;
    beginBatch(batchState);
    batchState.issues = [];
    batchState.deferredPages = 0;
    batchState.noCandidatePages = 0;
    state.maintenance.activity.current = retryFailed ? retryStart + progressIndex : maintenanceProcessedCalls();
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Summary")} ${formatNumber(index + 1)} / ${formatNumber(batches.length)}`;
    renderMaintenanceSession();
    try {
      const result = await maintenanceMutation("/api/maintenance/summaries/analyze-batch", {
        scanId: scan.scanId,
        pages: batch.map((item) => ({ pageId: item.pageId, revisionId: item.revisionId })),
      });
      replaceMaintenanceBatchCandidates("summary", batchState, result.candidates);
      analysis.analyzedAt = result.analyzedAt;
      batchState.workerCalls += result.workerCalls || 0;
      batchState.noCandidatePages = result.noCandidatePages || 0;
      batchState.deferredPages = result.deferredPages || 0;
      batchState.issues = (result.issues || []).map((issue) => ({
        ...issue,
        batchIndex: index,
        message: issue.message || String(issue),
      }));
      if (batchState.issues.length) failBatch(batchState, batchState.issues[0].message);
      else completeBatch(batchState);
    } catch (error) {
      batchState.workerCalls += 1;
      batchState.deferredPages = batch.length;
      batchState.issues = [{ batchIndex: index, message: error.message || String(error) }];
      failBatch(batchState, error);
    }
    refreshSummaryAnalysisTotals(analysis);
    if (retryFailed) state.maintenance.activity.current = retryStart + progressIndex + 1;
    else syncMaintenanceAnalyzeProgress();
    renderMaintenanceSession();
  }
}

async function analyzeRelationPhase(scan, { retryFailed = false } = {}) {
  let analysis = state.maintenance.analyses.relation;
  if (!retryFailed || !analysis) {
    analysis = emptyRelationAnalysis(scan);
    state.maintenance.analyses.relation = analysis;
  }
  const indexes = runnableBatchIndexes(analysis.batches, { retryFailed });
  const retryStart = retryFailed ? state.maintenance.activity.current : 0;
  for (const [progressIndex, index] of indexes.entries()) {
    const group = scan.groups[index];
    const batch = analysis.batches[index];
    if (!group || !batch) continue;
    beginBatch(batch);
    batch.decision = null;
    batch.issue = null;
    analysis.issues = analysis.issues.filter((issue) => issue.batchIndex !== index);
    state.maintenance.activity.current = retryFailed ? retryStart + progressIndex : maintenanceProcessedCalls();
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Relations")} ${formatNumber(index + 1)} / ${formatNumber(scan.groups.length)}`;
    renderMaintenanceSession();
    try {
      const result = await maintenanceMutation("/api/maintenance/relations/analyze", {
        scanId: scan.scanId,
        groupId: group.groupId,
      });
      analysis.analyzedAt = result.analyzedAt;
      batch.workerCalls = (batch.workerCalls || 0) + 1;
      if (result.candidate) {
        replaceMaintenanceBatchCandidates("relation", batch, [result.candidate]);
        batch.decision = "candidate";
      } else if (result.decision === "defer") {
        replaceMaintenanceBatchCandidates("relation", batch, []);
        batch.decision = "defer";
      } else {
        replaceMaintenanceBatchCandidates("relation", batch, []);
        batch.decision = result.decision || "none";
      }
      completeBatch(batch);
    } catch (error) {
      batch.workerCalls = (batch.workerCalls || 0) + 1;
      failBatch(batch, error);
    }
    refreshGroupAnalysisTotals(analysis);
    if (retryFailed) state.maintenance.activity.current = retryStart + progressIndex + 1;
    else syncMaintenanceAnalyzeProgress();
    renderMaintenanceSession();
  }
}

async function analyzeTopicPhase(scan, { retryFailed = false } = {}) {
  let analysis = state.maintenance.analyses.topic;
  if (!retryFailed || !analysis || analysis.scanId !== scan.scanId) {
    analysis = emptyTopicAnalysis(scan);
    state.maintenance.analyses.topic = analysis;
  }
  const indexes = runnableBatchIndexes(analysis.batches, { retryFailed });
  const retryStart = retryFailed ? state.maintenance.activity.current : 0;
  for (const [progressIndex, index] of indexes.entries()) {
    const group = scan.groups[index];
    const batch = analysis.batches[index];
    if (!group || !batch) continue;
    beginBatch(batch);
    batch.decision = null;
    state.maintenance.activity.current = retryFailed ? retryStart + progressIndex : maintenanceProcessedCalls();
    byId("maintenance-status").textContent = `${t("Analyzing")} ${t("Extract Topic Page")} ${formatNumber(index + 1)} / ${formatNumber(scan.groups.length)}`;
    renderMaintenanceSession();
    try {
      const result = await maintenanceMutation("/api/maintenance/topics/analyze", {
        scanId: scan.scanId,
        groupId: group.groupId,
      });
      analysis.analyzedAt = result.analyzedAt;
      batch.workerCalls += 1;
      if (result.candidate) {
        replaceMaintenanceBatchCandidates("topic", batch, [result.candidate]);
        batch.decision = "candidate";
      } else {
        replaceMaintenanceBatchCandidates("topic", batch, []);
        batch.decision = result.decision === "defer" ? "defer" : (result.decision || "none");
      }
      completeBatch(batch);
    } catch (error) {
      batch.workerCalls += 1;
      failBatch(batch, error);
    }
    refreshGroupAnalysisTotals(analysis);
    if (retryFailed) state.maintenance.activity.current = retryStart + progressIndex + 1;
    else syncMaintenanceAnalyzeProgress();
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
      else if (phase === "topic") await analyzeTopicPhase(scan);
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

async function retryFailedMaintenanceBatches() {
  if (state.maintenance.busy || maintenanceWorkflowStage() !== "review") return;
  const packFailures = packFailedBatches();
  const summaryFailures = summaryFailedBatches();
  const relationFailures = relationFailedBatches();
  const topicFailures = topicFailedBatches();
  if (packFailures.length + summaryFailures.length + relationFailures.length + topicFailures.length === 0) return;
  state.maintenance.busy = true;
  state.maintenance.activity = {
    kind: "analyze",
    current: 0,
    total: packFailures.length + summaryFailures.length + relationFailures.length + topicFailures.length,
    retry: true,
  };
  byId("maintenance-status").textContent = t("Preparing");
  renderMaintenanceSession();
  try {
    const previousCalls = new Map(maintenancePassPhases().map((phase) => [
      phase,
      state.maintenance.analyses[phase]?.workerCalls || 0,
    ]));
    if (packFailures.length) {
      const scan = maintenanceScanForPhase("pack");
      if (scan) await analyzePackingPhase(scan, { retryFailed: true });
    }
    if (summaryFailures.length) {
      const scan = maintenanceScanForPhase("summary");
      if (scan) await analyzeSummaryPhase(scan, { retryFailed: true });
    }
    if (relationFailures.length) {
      const scan = maintenanceScanForPhase("relation");
      if (scan) await analyzeRelationPhase(scan, { retryFailed: true });
    }
    if (topicFailures.length) {
      const scan = maintenanceScanForPhase("topic");
      if (scan) await analyzeTopicPhase(scan, { retryFailed: true });
    }
    for (const phase of maintenancePassPhases()) {
      const analysis = state.maintenance.analyses[phase];
      const outcome = maintenanceOutcome(phase);
      if (!analysis || !outcome) continue;
      outcome.analyzedAt = analysis.analyzedAt || outcome.analyzedAt || new Date().toISOString();
      outcome.modelCalls += Math.max(0, (analysis.workerCalls || 0) - (previousCalls.get(phase) || 0));
      outcome.proposals = state.maintenance.pendingCandidates
        .filter((candidate) => candidate.operation === phase).length;
      outcome.issues = [...(analysis.issues || [])];
    }
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
  const { candidates, rejections, suppressions } = maintenanceApplySelection();
  const totalCandidates = [...candidates, ...rejections, ...suppressions];
  if (totalCandidates.length === 0) return;
  const candidateCount = formatNumber(totalCandidates.length);
  const singular = totalCandidates.length === 1;
  const phaseLabel = t(maintenancePhaseConfig().label);
  const confirmed = await confirmAction({
    title: currentLanguage === "zh"
      ? `应用 ${candidateCount} 个${phaseLabel}决策？`
      : `Apply ${candidateCount} ${phaseLabel} decision${singular ? "" : "s"}?`,
    description: currentLanguage === "zh"
      ? "会提交已选择的变更、接受或拒绝的关联，以及标为不再建议的关联；本次跳过的项目不会写入。每项都会在写入前重新校验当前版本。"
      : "Selected changes, accepted or rejected relations, and staged suppressions will be committed. Items skipped for now are left untouched. Every decision revalidates the current revision before writing.",
    confirmLabel: t("Apply decisions"),
  });
  if (!confirmed) return;

  state.maintenance.busy = true;
  state.maintenance.activity = { kind: "optimize", current: 0, total: totalCandidates.length };
  byId("maintenance-status").textContent = `Optimizing 0 of ${formatNumber(totalCandidates.length)}`;
  renderMaintenanceSession();
  let result = { applied: 0, appliedCandidateIds: [], skipped: [] };
  try {
    const reportProgress = (index, applied, skipped) => {
      state.maintenance.activity.current = index;
      byId("maintenance-status").textContent = `Optimizing ${formatNumber(index)} of ${formatNumber(totalCandidates.length)} · ${formatNumber(applied)} applied · ${formatNumber(skipped.length)} skipped`;
      renderMaintenanceSession();
    };
    const reflectCandidateProgress = ({ candidate, status, error }) => {
      if (!candidate) return;
      if (status === "applied") {
        state.maintenance.applyStates.delete(candidate.candidateId);
        state.maintenance.selected.delete(candidate.candidateId);
        state.maintenance.pendingCandidates = state.maintenance.pendingCandidates
          .filter((item) => item.candidateId !== candidate.candidateId);
        const packAnalysis = state.maintenance.analyses.pack;
        if (packAnalysis) packAnalysis.candidates = packAnalysis.candidates
          .filter((item) => item.candidateId !== candidate.candidateId);
        state.maintenance.relationDraftStates.delete(candidate.candidateId);
        state.maintenance.relationReviewStates.delete(candidate.candidateId);
        state.maintenance.candidateReviewStates.delete(candidate.candidateId);
      } else {
        state.maintenance.applyStates.set(candidate.candidateId, { status, message: error || null });
      }
    };
    const appliedResult = await applyMaintenanceCandidates(candidates, ({ index, applied, skipped, candidate, status, error }) => {
      reflectCandidateProgress({ candidate, status, error });
      reportProgress(index, applied, skipped);
    });
    const rejectionResult = await applyRelationRejections(rejections, ({ index, applied, skipped, candidate, status, error }) => {
      reflectCandidateProgress({ candidate, status, error });
      reportProgress(candidates.length + index, appliedResult.applied + applied, [
        ...appliedResult.skipped,
        ...skipped,
      ]);
    });
    const suppressionResult = await applyRelationSuppressions(suppressions, ({ index, applied, skipped, candidate, status, error }) => {
      reflectCandidateProgress({ candidate, status, error });
      reportProgress(candidates.length + rejections.length + index, appliedResult.applied + rejectionResult.applied + applied, [
        ...appliedResult.skipped,
        ...rejectionResult.skipped,
        ...skipped,
      ]);
    });
    result = {
      applied: appliedResult.applied + rejectionResult.applied + suppressionResult.applied,
      appliedCandidateIds: [
        ...appliedResult.appliedCandidateIds,
        ...rejectionResult.appliedCandidateIds,
        ...suppressionResult.appliedCandidateIds,
      ],
      skipped: [
        ...appliedResult.skipped,
        ...rejectionResult.skipped,
        ...suppressionResult.skipped,
      ],
    };
    const byCandidateId = new Map(totalCandidates.map((candidate) => [candidate.candidateId, candidate]));
    for (const candidateId of appliedResult.appliedCandidateIds) {
      const candidate = byCandidateId.get(candidateId);
      if (candidate) maintenanceOutcome(candidate.operation).applied += 1;
    }
    maintenanceOutcome("relation").rejected += rejectionResult.applied;
    maintenanceOutcome("relation").suppressed += suppressionResult.applied;
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

async function runMaintenanceConvergence() {
  if (state.maintenance.busy || !maintenanceAvailable() || archiveSessionActive()) return;
  state.maintenance.busy = true;
  state.maintenance.activity = t("Working");
  state.maintenance.convergence = { running: true, report: null, steps: 0, completedAt: null, error: null };
  renderMaintenanceSession();
  renderAutomationStatus();
  const maxSteps = 512;
  try {
    for (let step = 0; step < maxSteps; step += 1) {
      state.maintenance.activity = currentLanguage === "zh"
        ? `正在推进第 ${formatNumber(step + 1)} 个工作单元`
        : `Advancing bounded job ${formatNumber(step + 1)}`;
      byId("maintenance-status").textContent = state.maintenance.activity;
      const response = await maintenanceMutation("/api/maintenance/converge", {});
      state.maintenance.convergence.steps += response.report?.jobsAdvanced || 0;
      state.maintenance.convergence.report = mergeConvergenceReport(
        state.maintenance.convergence.report,
        response.report || {},
      );
      state.maintenance.relationReviews = response.reviews || [];
      renderRelationReviews();
      renderAutomationStatus();
      await new Promise((resolve) => window.requestAnimationFrame(resolve));
      if (convergenceSettled(response)) {
        state.maintenance.convergence.running = false;
        state.maintenance.convergence.completedAt = new Date().toISOString();
        break;
      }
    }
    if (state.maintenance.convergence.running) {
      state.maintenance.convergence.running = false;
      throw new Error(currentLanguage === "zh"
        ? "本次已达到 512 个工作单元的安全上限；可以再次点击立即运行继续收敛。"
        : "This run reached the 512-job safety bound; choose Run now again to continue convergence.");
    }
    await Promise.all([loadRelationReviews(), loadOverview()]);
    renderMaintenanceStatus(await api("/api/maintenance"));
  } catch (error) {
    state.maintenance.convergence.running = false;
    state.maintenance.convergence.error = {
      message: error.message || String(error),
      occurredAt: new Date().toISOString(),
    };
  } finally {
    state.maintenance.busy = false;
    state.maintenance.activity = null;
    renderMaintenanceSession();
    renderAutomationStatus();
  }
}

async function startMaintenanceSession() {
  if (state.maintenance.busy || !maintenanceAvailable() || archiveSessionActive()) return;
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
  const unresolved = maintenanceUnresolvedCandidates();
  if (unresolved.length > 0) {
    const confirmed = await confirmAction({
      title: currentLanguage === "zh" ? "跳过未应用的提案？" : "Continue without applying remaining proposals?",
      description: currentLanguage === "zh"
        ? `本段还有 ${formatNumber(unresolved.length)} 个未审提案。它们会按跳过处理且不会写入；下一维护段会重新扫描实际库存。`
        : `${formatNumber(unresolved.length)} proposals remain unreviewed. They will be skipped and not written; the next maintenance pass will rescan the actual inventory.`,
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
  const passOrder = maintenancePassConfig().order;
  const nextPass = Object.entries(MAINTENANCE_PASSES)
    .find(([, config]) => config.order === passOrder + 1)?.[0];
  if (nextPass) {
    state.maintenance.pass = nextPass;
    state.maintenance.phase = maintenancePassPhases(nextPass)[0];
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
  if (action === "apply") return optimizeMaintenanceSelection();
  return advanceMaintenancePhase();
}

async function activateView(name, { reload = false } = {}) {
  state.activeView = name;
  scheduleMaintenanceStatusPoll();
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
  if (state.governance.loaded) renderGovernance({ hits: state.governance.hits, nextCursor: state.governance.cursor });
  queryView.rerender();
  if (state.maintenance.loaded) {
    renderAutomationStatus();
    renderMaintenanceSession();
    renderRelationReviews();
  }
  renderArchiveSession();
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
byId("governance-scope").addEventListener("change", (event) => {
  resetGovernance({ status: "archived", scope: event.target.value });
  loadGovernance().catch(showError);
});
byId("governance-refresh").addEventListener("click", () => loadGovernance({ reload: true }).catch(showError));
byId("governance-more").addEventListener("click", () => loadGovernance({ append: true }).catch(showError));
byId("maintenance-archive-library").addEventListener("toggle", (event) => {
  if (event.target.open) loadGovernance().catch(showError);
});
document.addEventListener("click", (event) => {
  if (!event.target.closest(".page-control-menu")) closePageMenus();
});
byId("pages-previous").addEventListener("click", () => loadPages({ page: state.pages.page - 1 }).catch(showError));
byId("pages-next").addEventListener("click", () => loadPages({ page: state.pages.page + 1 }).catch(showError));
byId("maintenance-start").addEventListener("click", () => runMaintenanceConvergence().catch(showError));
byId("maintenance-scene-alert-retry").addEventListener("click", () => runMaintenanceConvergence().catch(showError));
byId("maintenance-review-undo-all").addEventListener("click", undoAllMaintenanceReviews);
byId("maintenance-review-commit").addEventListener("click", () => commitMaintenanceReviewSession().catch(showError));
byId("maintenance-manual-start").addEventListener("click", () => startMaintenanceSession().catch(showError));
byId("maintenance-primary").addEventListener("click", () => runMaintenancePrimaryAction().catch(showError));
byId("maintenance-skip").addEventListener("click", () => skipMaintenancePhase().catch(showError));
byId("maintenance-retry-failed").addEventListener("click", () => retryFailedMaintenanceBatches().catch(showError));
byId("maintenance-rescan").addEventListener("click", () => rescanMaintenancePhase().catch(showError));
byId("maintenance-cancel").addEventListener("click", () => cancelMaintenanceSession().catch(showError));
byId("maintenance-start-new").addEventListener("click", () => runMaintenanceConvergence().catch(showError));
byId("archive-start").addEventListener("click", () => startArchiveSession().catch(showError));
byId("archive-analyze").addEventListener("click", () => analyzeArchiveCandidates().catch(showError));
byId("archive-retry-failed").addEventListener("click", () => analyzeArchiveCandidates({ retryFailed: true }).catch(showError));
byId("archive-apply").addEventListener("click", () => applyArchiveSelection().catch(showError));
byId("archive-rescan").addEventListener("click", () => scanArchiveCandidates().catch(showError));
byId("archive-finish").addEventListener("click", () => finishArchiveSession().catch(showError));
byId("archive-start-new").addEventListener("click", () => startArchiveSession().catch(showError));
byId("maintenance-settings-form").addEventListener("submit", (event) => saveMaintenanceSettings(event).catch(showError));
byId("access-more").addEventListener("click", () => loadAccess({ append: true }).catch(showError));
byId("health-window").addEventListener("change", () => healthView.load({ reload: true }).catch(showError));
refresh();
loadEnrollment();
window.setInterval(() => loadEnrollment(), 3000);
