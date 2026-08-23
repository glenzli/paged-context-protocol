import { createPageInspector } from "/page-inspector.js?v=20260823.1";
import { describePagePayload, pagePayloadPreviewText } from "/page-content.js?v=20260822.1";
import { createHealthView } from "/health-view.js?v=20260816.3";
import { createRetentionView } from "/retention-view.js?v=20260818.1";
import { createQueryView } from "/query-view.js?v=20260823.2";
import {
  batchProgress,
  beginBatch,
  completeBatch,
  failBatch,
  runnableBatchIndexes,
} from "/progressive-operation.js?v=20260823.2";

const DEFAULT_PAGE_LIMIT = 20;
const PAGE_LIMIT_OPTIONS = new Set([10, 20, 30]);
const ACCESS_LIMIT = 50;
// Local summary workers receive one Page at a time so an incomplete response
// affects only that Page and can be retried independently.
const SUMMARY_REVIEW_BATCH_SIZE = 1;
const THEME_STORAGE_KEY = "pcp-console.theme";
const LANGUAGE_STORAGE_KEY = "pcp-console.language";
const PAGE_LIMIT_STORAGE_KEY = "pcp-console.pages-per-page";
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
  "Maintain Pack boundaries, semantic structure, then topic Pages in order. Suggested links are never retrievable before approval.": "按顺序维护 Pack 边界、语义结构与主题页；建议关联仍需批准后才会参与检索。",
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
  "Awaiting the first Runtime heartbeat.": "等待 Runtime 首次心跳。",
  "Approve": "批准",
  "Accept": "接受",
  "Accepted": "已接受",
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
  "Reviewed": "已审阅",
  "Mark reviewed": "标记已审阅",
  "Rejected for this review": "本次拒绝",
  "Undo rejection": "撤销拒绝",
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
    applyStates: new Map(),
    relationReviews: [],
  },
  archive: {
    state: "idle",
    busy: false,
    activity: null,
    operation: null,
    scan: null,
    analyses: [],
    selected: new Set(),
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

function openPageIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M5 19 19 5M8 5h11v11");
  svg.append(path);
  return svg;
}

function relationCompareIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  for (const d of ["M4 5h6v14H4z", "M14 5h6v14h-6z", "M10 12h4", "m2-2 2 2-2 2"]) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

function suppressRelationIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  for (const d of [
    "m9 15-1.4 1.4a3 3 0 0 1-4.2-4.2l3.5-3.5a3 3 0 0 1 4.2 0L12.5 10",
    "m15 9 1.4-1.4a3 3 0 0 1 4.2 4.2l-3.5 3.5a3 3 0 0 1-4.2 0L11.5 14",
    "M4 4 20 20",
  ]) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

function acceptRelationIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  for (const d of ["M20 6 9 17l-5-5", "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20"]) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

function rejectRelationIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  for (const d of ["M8 8l8 8M16 8l-8 8", "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20"]) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
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
  const card = element("article", "archive-candidate-card");
  const heading = element("div", "archive-candidate-heading");
  const select = document.createElement("input");
  select.type = "checkbox";
  select.checked = state.archive.selected.has(candidate.candidateId);
  select.disabled = state.archive.operation === "apply";
  select.setAttribute("aria-label", t("Select archive candidate"));
  select.addEventListener("change", () => {
    if (select.checked) state.archive.selected.add(candidate.candidateId);
    else state.archive.selected.delete(candidate.candidateId);
    renderArchiveSession();
  });
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
  heading.append(select, copy, open);
  const preview = element("p", "archive-candidate-preview", candidate.preview);
  const reason = element("p", "archive-candidate-reason", candidate.reason);
  const signals = element("div", "archive-candidate-signals");
  candidate.candidateSignals.forEach((signal) => signals.append(element("span", "status-pill", signal)));
  if (candidate.applyIssue) {
    const failed = element("span", "maintenance-apply-state failed", currentLanguage === "zh" ? "应用失败，可重试" : "Apply failed; retry available");
    failed.title = candidate.applyIssue;
    signals.append(failed);
  }
  card.append(heading, preview, reason, signals);
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
        ? `分析仍在继续 · 已选 ${formatNumber(selectedCount)} / ${formatNumber(candidates.length)} · 完成前不能应用`
        : `Analysis is still running · ${formatNumber(selectedCount)} of ${formatNumber(candidates.length)} selected · applying is locked`)
      : `${t("Selected")} ${formatNumber(selectedCount)} / ${formatNumber(candidates.length)}`;
    const selectAll = byId("archive-select-all");
    selectAll.checked = candidates.length > 0 && selectedCount === candidates.length;
    selectAll.indeterminate = selectedCount > 0 && selectedCount < candidates.length;
    selectAll.disabled = state.archive.busy || stage !== "review" || candidates.length === 0;
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

function toggleArchiveSelection() {
  const candidates = archiveCandidates();
  const allSelected = candidates.length > 0 && candidates.every((candidate) => state.archive.selected.has(candidate.candidateId));
  state.archive.selected.clear();
  if (!allSelected) candidates.forEach((candidate) => state.archive.selected.add(candidate.candidateId));
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

function finishArchiveSession() {
  if (!archiveSessionActive() || state.archive.busy) return;
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
  state.maintenance.applyStates.clear();
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
    metric(t("Pending relation review"), formatNumber(automation.pendingRelationReviewCount), automation.pendingRelationReviewCount ? "warning" : ""),
  );
  const currentReport = automation.currentReport;
  const progress = byId("maintenance-automation-progress");
  progress.hidden = !currentReport;
  if (currentReport) {
    const proposed = (currentReport.summariesProposed || 0)
      + (currentReport.packsProposed || 0)
      + (currentReport.relationsProposed || 0)
      + (currentReport.retentionLeasesProposed || 0);
    const committed = (currentReport.summariesWritten || 0)
      + (currentReport.packsCommitted || 0)
      + (currentReport.relationsCommitted || 0)
      + (currentReport.retentionLeasesWritten || 0);
    byId("maintenance-automation-progress-title").textContent = automation.state === "running"
      ? (currentLanguage === "zh" ? "本轮实时进度" : "Live cycle progress")
      : (currentLanguage === "zh" ? "上次失败前的部分进度" : "Partial progress before the last failure");
    byId("maintenance-automation-progress-metrics").replaceChildren(
      metric(t("Maintenance inventory"), formatNumber(currentReport.inspectedPages)),
      metric(t("Model calls"), formatNumber(currentReport.workerCalls), currentReport.workerCalls ? "info" : ""),
      metric(t("Proposals"), formatNumber(proposed), proposed ? "warning" : ""),
      metric(t("Applied"), formatNumber(committed), committed ? "positive" : "", `${formatNumber(currentReport.deferred || 0)} ${t("Deferred")}`),
    );
  }
  const completed = automation.lastCompletedAt;
  const started = automation.lastStartedAt;
  byId("maintenance-automation-detail").textContent = started || completed
    ? [started ? `${currentLanguage === "zh" ? "最近开始" : "Last started"}: ${formatTime(started)}` : "", completed ? `${t("Last completed")}: ${formatTime(completed)}` : ""].filter(Boolean).join(" · ")
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

function relationComparisonButton(proposal, className = "secondary-button", {
  iconOnly = false,
  onReviewed = null,
  reviewed = false,
  onAccept = null,
  onReject = null,
  accepted = false,
  rejected = false,
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
      onReviewed,
      reviewed,
      onAccept,
      onReject,
      accepted,
      rejected,
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

function relationReviewCard(proposal) {
  const card = element("article", "maintenance-relation-review-card");
  const heading = element("div", "maintenance-relation-review-card-heading");
  const annotations = [];
  heading.append(
    element("strong", "", t("Review evidence")),
    element("span", "maintenance-relation-review-risk", t("Manual approval required")),
    element("span", "muted", formatTime(proposal.proposedAt)),
  );
  if (proposal.relationReason) {
    const evidence = element("div", "maintenance-relation-evidence");
    evidence.append(
      element("span", "maintenance-relation-evidence-label", t("Relation evidence")),
      element("span", "", proposal.relationReason),
    );
    annotations.push(evidence);
  }
  if (proposal.reviewReason) {
    annotations.push(element("p", "maintenance-relation-review-reason muted", proposal.reviewReason));
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
    pageCard.addEventListener("click", () => {
      pageInspector.compareRelation({
        pages: proposal.pages,
        relationReason: proposal.relationReason,
        reviewReason: proposal.reviewReason,
      }).catch(() => {});
    });
    pageColumn.append(pageCard);
    pages.append(pageColumn);
    if (index === 0) pages.append(element("span", "maintenance-relation-review-link", "↔"));
  });
  const actions = element("div", "maintenance-relation-review-actions");
  const approve = element("button", "primary-button", t("Approve"));
  const reject = element("button", "warning-button", t("Reject"));
  const suppress = element("button", "danger-button", t("Do not suggest this relation again"));
  [approve, reject, suppress].forEach((button) => { button.type = "button"; });
  approve.addEventListener("click", () => resolveRelationReview(proposal.candidateId, "approve"));
  reject.addEventListener("click", () => resolveRelationReview(proposal.candidateId, "reject"));
  suppress.addEventListener("click", () => resolveRelationReview(proposal.candidateId, "suppress"));
  actions.append(relationComparisonButton(proposal, "compact-button compact-icon-button", {
    iconOnly: true,
    onAccept: () => resolveRelationReview(proposal.candidateId, "approve"),
    onReject: () => resolveRelationReview(proposal.candidateId, "reject"),
  }), approve, reject, suppress);
  card.append(heading, ...annotations, pages, actions);
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

function maintenanceSelectionCheckbox(candidate) {
  const checkbox = document.createElement("input");
  checkbox.type = "checkbox";
  const stagedSuppression = candidate.operation === "relation"
    && relationDraftState(candidate) === "suppressed";
  checkbox.checked = !stagedSuppression && state.maintenance.selected.has(candidate.candidateId);
  checkbox.disabled = stagedSuppression || state.maintenance.applyStates.get(candidate.candidateId)?.status === "running";
  checkbox.setAttribute("aria-label", `Select ${candidate.candidateId}`);
  if (stagedSuppression) checkbox.title = t("Will not be suggested when applied");
  checkbox.addEventListener("change", () => {
    if (checkbox.checked) state.maintenance.selected.add(candidate.candidateId);
    else state.maintenance.selected.delete(candidate.candidateId);
    if (candidate.operation === "relation") {
      state.maintenance.relationReviewStates.set(
        candidate.candidateId,
        checkbox.checked ? "accepted" : "rejected",
      );
    }
    renderMaintenanceSession();
  });
  return checkbox;
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

function markRelationCandidateReviewed(candidate) {
  if (candidate.operation !== "relation" || relationDraftState(candidate) === "suppressed") return;
  state.maintenance.relationReviewStates.set(candidate.candidateId, "reviewed");
  renderMaintenanceSession();
}

function acceptRelationCandidate(candidate) {
  if (candidate.operation !== "relation" || relationDraftState(candidate) === "suppressed") return;
  state.maintenance.selected.add(candidate.candidateId);
  state.maintenance.relationReviewStates.set(candidate.candidateId, "accepted");
  renderMaintenanceSession();
}

function rejectRelationCandidate(candidate) {
  if (candidate.operation !== "relation" || relationDraftState(candidate) === "suppressed") return;
  state.maintenance.selected.delete(candidate.candidateId);
  state.maintenance.relationReviewStates.set(candidate.candidateId, "rejected");
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

function stagedRelationSuppressions() {
  return maintenanceCandidates().filter((candidate) => (
    candidate.operation === "relation" && relationDraftState(candidate) === "suppressed"
  ));
}

function maintenanceApplySelection() {
  const suppressions = stagedRelationSuppressions();
  const suppressionIds = new Set(suppressions.map((candidate) => candidate.candidateId));
  const candidates = maintenanceCandidates().filter((candidate) => (
    state.maintenance.selected.has(candidate.candidateId) && !suppressionIds.has(candidate.candidateId)
  ));
  return { candidates, suppressions };
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
  } else if (candidate.operation === "topic") {
    source.append(
      element("strong", "", candidate.namespace),
      element("span", "muted", `${candidate.pages.length} ${t("Pages")}`),
    );
    change.append(
      element("strong", "", t("Extract Topic Page")),
      element("span", "muted", candidate.title),
    );
    inputs.append(
      element("span", "mono muted", candidate.pages.map((page) => page.pageId).join(" · ")),
      topicExtractionReviewButton(candidate),
    );
    if (candidate.reason) {
      const rationale = element("div", "maintenance-relation-evidence");
      rationale.append(
        element("span", "maintenance-relation-evidence-label", t("Why this Topic Page is worth creating")),
        element("span", "", candidate.reason),
      );
      content.append(rationale);
    }
    content.append(element("span", "maintenance-preview", compactPreview(candidate.content, 240)));
  } else if (candidate.operation === "relation") {
    const draftState = relationDraftState(candidate);
    source.append(
      element("strong", "", candidate.namespace),
      element("span", "muted", `2 ${t("Pages")}`),
    );
    change.append(
      element("strong", "", t("Explicit relation")),
      element("span", "muted", "related_to"),
    );
    if (draftState === "reviewed") {
      change.append(element("span", "maintenance-relation-state reviewed", t("Reviewed")));
    } else if (draftState === "suppressed") {
      change.append(element("span", "maintenance-relation-state suppressed", t("Will not be suggested when applied")));
    }
    for (const page of candidate.pages) {
      const item = element("div", "maintenance-input");
      item.append(
        element("span", "mono muted", page.pageId),
        element("span", "maintenance-preview", compactPreview(page.preview || t("No preview"))),
      );
      inputs.append(item);
    }
    content.append(element("strong", "", "related_to"));
    if (candidate.relationReason) {
      const evidence = element("div", "maintenance-relation-evidence");
      evidence.append(
        element("span", "maintenance-relation-evidence-label", t("Relation evidence")),
        element("span", "", candidate.relationReason),
      );
      content.append(evidence);
    }
    const actions = element("div", "maintenance-relation-actions");
    const isSuppressed = draftState === "suppressed";
    const suppressLabel = t(isSuppressed ? "Undo no-suggest" : "Do not suggest this relation again");
    const suppress = element("button", `icon-button maintenance-relation-action-button maintenance-relation-suppress${isSuppressed ? " is-staged" : ""}`);
    suppress.type = "button";
    suppress.title = suppressLabel;
    suppress.setAttribute("aria-label", suppressLabel);
    suppress.append(suppressRelationIcon());
    suppress.addEventListener("click", () => toggleRelationCandidateSuppression(candidate));
    actions.append(
      relationComparisonButton(candidate, "icon-button maintenance-relation-action-button maintenance-relation-compare", {
        iconOnly: true,
        reviewed: draftState === "reviewed",
        onReviewed: draftState === "suppressed" ? null : () => markRelationCandidateReviewed(candidate),
      }),
      suppress,
    );
    content.append(actions);
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
  const applyState = maintenanceApplyStateNode(candidate);
  if (applyState) change.append(applyState);
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
  const open = element("button", "compact-button compact-icon-button");
  open.type = "button";
  open.title = t("Open page");
  open.setAttribute("aria-label", t("Open page"));
  open.append(openPageIcon());
  open.addEventListener("click", () => pageInspector.open(candidate.pageId));
  heading.append(selection, source, open);
  const metadata = element("div", "maintenance-summary-metadata");
  metadata.textContent = `${t("Summary route")} · ${formatSize(candidate.contentChars)} ${t("Page")}`;
  const applyState = maintenanceApplyStateNode(candidate);
  if (applyState) metadata.append(" · ", applyState);
  card.append(heading, metadata, element("div", "maintenance-summary-content", candidate.content));
  return card;
}

function relationProposalCard(candidate) {
  const draftState = relationDraftState(candidate);
  const reviewState = relationReviewState(candidate);
  const card = element("article", `maintenance-relation-proposal${reviewState ? ` is-${reviewState}` : ""}${draftState === "suppressed" ? " is-suppressed" : ""}`);
  const aside = element("div", "maintenance-relation-proposal-aside");
  const selection = element("label", "maintenance-card-select");
  selection.append(maintenanceSelectionCheckbox(candidate), element("span", "", t("Proposals")));
  aside.append(
    selection,
    element("strong", "", candidate.namespace),
    element("span", "muted", `${candidate.pages.length} ${t("Pages")}`),
  );
  if (reviewState === "accepted") aside.append(element("span", "maintenance-relation-state accepted", t("Accepted")));
  else if (reviewState === "rejected") aside.append(element("span", "maintenance-relation-state rejected", t("Rejected for this review")));
  else if (reviewState === "reviewed") aside.append(element("span", "maintenance-relation-state reviewed", t("Reviewed")));
  if (draftState === "suppressed") aside.append(element("span", "maintenance-relation-state suppressed", t("Will not be suggested when applied")));
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
    reviewed: reviewState === "reviewed",
    onReviewed: draftState === "suppressed" ? null : () => markRelationCandidateReviewed(candidate),
    onAccept: draftState === "suppressed" ? null : () => acceptRelationCandidate(candidate),
    onReject: draftState === "suppressed" ? null : () => rejectRelationCandidate(candidate),
    accepted: reviewState === "accepted",
    rejected: reviewState === "rejected",
  });
  view.title = t("View relation Pages");
  view.setAttribute("aria-label", view.title);
  view.replaceChildren(openPageIcon());
  const accept = element("button", `icon-button maintenance-relation-action-button maintenance-relation-accept${reviewState === "accepted" ? " is-active" : ""}`);
  accept.type = "button";
  accept.title = t("Accept");
  accept.setAttribute("aria-label", t("Accept"));
  accept.append(acceptRelationIcon());
  accept.disabled = draftState === "suppressed";
  accept.addEventListener("click", () => acceptRelationCandidate(candidate));
  const reject = element("button", `icon-button maintenance-relation-action-button maintenance-relation-reject${reviewState === "rejected" ? " is-active" : ""}`);
  reject.type = "button";
  reject.title = t("Reject");
  reject.setAttribute("aria-label", t("Reject"));
  reject.append(rejectRelationIcon());
  reject.disabled = draftState === "suppressed";
  reject.addEventListener("click", () => rejectRelationCandidate(candidate));
  actions.append(view, accept, reject);
  if (reviewState === "rejected" || draftState === "suppressed") {
    const suppress = element("button", "maintenance-relation-no-suggest", t(draftState === "suppressed" ? "Undo no-suggest" : "Do not suggest this relation again"));
    suppress.type = "button";
    suppress.addEventListener("click", () => toggleRelationCandidateSuppression(candidate));
    actions.append(suppress);
  }
  body.append(pages, rationale, actions);
  card.append(aside, body);
  return card;
}

function renderMaintenanceProposals(candidates) {
  const summaryCandidates = candidates.filter((candidate) => candidate.operation === "summary");
  const relationCandidates = candidates.filter((candidate) => candidate.operation === "relation");
  const tableCandidates = candidates.filter((candidate) => candidate.operation !== "summary" && candidate.operation !== "relation");
  const summaryCards = byId("maintenance-summary-cards");
  const relationCards = byId("maintenance-relation-cards");
  const table = byId("maintenance-table-wrap");
  summaryCards.hidden = summaryCandidates.length === 0;
  relationCards.hidden = relationCandidates.length === 0;
  table.hidden = tableCandidates.length === 0;
  if (summaryCandidates.length) summaryCards.replaceChildren(...summaryCandidates.map(summaryProposalCard));
  if (relationCandidates.length) relationCards.replaceChildren(...relationCandidates.map(relationProposalCard));
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
  if (maintenanceWorkflowStage() === "review" && maintenanceFailedBatches().length) return t("Analysis incomplete");
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
  if (maintenanceCandidates().length) return t("Review the proposals below, select the changes to apply, then continue to the next stage.");
  return t("Analysis completed. No changes are recommended for this stage. Continue when you are ready.");
}

function currentMaintenanceAction() {
  if (!maintenanceSessionActive()) return "start";
  const stage = maintenanceWorkflowStage();
  if (stage === "scan") return "scan";
  if (stage === "analyze") return "analyze";
  const { candidates, suppressions } = maintenanceApplySelection();
  return candidates.length + suppressions.length ? "apply" : "advance";
}

function maintenancePrimaryLabel(action = currentMaintenanceAction()) {
  if (action === "start") return t("Start maintenance");
  if (action === "scan") return t("Scan candidates");
  if (action === "analyze") return t("Analyze suggestions");
  if (action === "apply") {
    const { candidates, suppressions } = maintenanceApplySelection();
    const count = candidates.length + suppressions.length;
    return `${t("Apply selected")} (${formatNumber(count)})`;
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
  const selectedCount = candidates.filter((candidate) => state.maintenance.selected.has(candidate.candidateId)).length;

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
    ? `${formatNumber(candidates.length)} ${t("proposals")} · ${formatNumber(selectedCount)} ${currentLanguage === "zh" ? "已选" : "selected"}`
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
        ? `分析仍在继续 · 已选 ${formatNumber(selectedCount)} / ${formatNumber(candidates.length)} · 完成前不能应用`
        : `Analysis is still running · ${formatNumber(selectedCount)} of ${formatNumber(candidates.length)} selected · applying is locked`)
      : (currentLanguage === "zh"
        ? `已选 ${formatNumber(selectedCount)} / ${formatNumber(candidates.length)}`
        : `${formatNumber(selectedCount)} of ${formatNumber(candidates.length)} selected`);
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
  const selectAll = byId("maintenance-select-all");
  selectAll.disabled = state.maintenance.busy || candidates.length === 0;
  selectAll.checked = candidates.length > 0 && selectedCount === candidates.length;
  selectAll.indeterminate = selectedCount > 0 && selectedCount < candidates.length;

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
    metric(t("Relations"), `${formatNumber(outcomes.relation.applied)} ${t("Applied")}`, outcomes.relation.applied ? "positive" : ""),
    metric(t("Extract Topic Page"), `${formatNumber(outcomes.topic.applied)} ${t("Applied")}`, outcomes.topic.applied ? "positive" : ""),
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
  byId("maintenance-start").disabled = !maintenanceAvailable() || state.maintenance.busy || archiveSessionActive();
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
    else {
      state.maintenance.pendingCandidates.push(proposal);
      state.maintenance.selected.add(proposal.candidateId);
    }
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
  const { candidates, suppressions } = maintenanceApplySelection();
  const totalCandidates = [...candidates, ...suppressions];
  if (totalCandidates.length === 0) return;
  const candidateCount = formatNumber(totalCandidates.length);
  const singular = totalCandidates.length === 1;
  const phaseLabel = t(maintenancePhaseConfig().label);
  const confirmed = await confirmAction({
    title: currentLanguage === "zh"
      ? `优化选中的 ${candidateCount} 个${phaseLabel}提案？`
      : `Optimize ${candidateCount} selected ${phaseLabel} proposal${singular ? "" : "s"}?`,
    description: currentLanguage === "zh"
      ? "只会提交当前阶段中已选择的提案，以及标为不再建议的关联。每项都会在写入前校验当前版本；完成后由你决定何时继续到下一阶段。"
      : "Only selected proposals and staged relation suppressions in the current phase will be applied. Each write checks the current revision first; you choose when to continue to the next stage.",
    confirmLabel: t("Apply selected"),
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
      } else {
        state.maintenance.applyStates.set(candidate.candidateId, { status, message: error || null });
      }
    };
    const appliedResult = await applyMaintenanceCandidates(candidates, ({ index, applied, skipped, candidate, status, error }) => {
      reflectCandidateProgress({ candidate, status, error });
      reportProgress(index, applied, skipped);
    });
    const suppressionResult = await applyRelationSuppressions(suppressions, ({ index, applied, skipped, candidate, status, error }) => {
      reflectCandidateProgress({ candidate, status, error });
      reportProgress(candidates.length + index, appliedResult.applied + applied, [
        ...appliedResult.skipped,
        ...skipped,
      ]);
    });
    result = {
      applied: appliedResult.applied + suppressionResult.applied,
      appliedCandidateIds: [...appliedResult.appliedCandidateIds, ...suppressionResult.appliedCandidateIds],
      skipped: [...appliedResult.skipped, ...suppressionResult.skipped],
    };
    const byCandidateId = new Map(totalCandidates.map((candidate) => [candidate.candidateId, candidate]));
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
  const candidates = maintenanceCandidates().filter((candidate) => (
    relationDraftState(candidate) !== "suppressed"
  ));
  const allSelected = candidates.length > 0
    && candidates.every((candidate) => state.maintenance.selected.has(candidate.candidateId));
  state.maintenance.selected.clear();
  if (!allSelected) {
    for (const candidate of candidates) state.maintenance.selected.add(candidate.candidateId);
  }
  renderMaintenanceSession();
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
byId("maintenance-start").addEventListener("click", () => startMaintenanceSession().catch(showError));
byId("maintenance-primary").addEventListener("click", () => runMaintenancePrimaryAction().catch(showError));
byId("maintenance-skip").addEventListener("click", () => skipMaintenancePhase().catch(showError));
byId("maintenance-retry-failed").addEventListener("click", () => retryFailedMaintenanceBatches().catch(showError));
byId("maintenance-rescan").addEventListener("click", () => rescanMaintenancePhase().catch(showError));
byId("maintenance-cancel").addEventListener("click", () => cancelMaintenanceSession().catch(showError));
byId("maintenance-start-new").addEventListener("click", () => startMaintenanceSession().catch(showError));
byId("archive-start").addEventListener("click", () => startArchiveSession().catch(showError));
byId("archive-analyze").addEventListener("click", () => analyzeArchiveCandidates().catch(showError));
byId("archive-retry-failed").addEventListener("click", () => analyzeArchiveCandidates({ retryFailed: true }).catch(showError));
byId("archive-apply").addEventListener("click", () => applyArchiveSelection().catch(showError));
byId("archive-rescan").addEventListener("click", () => scanArchiveCandidates().catch(showError));
byId("archive-finish").addEventListener("click", finishArchiveSession);
byId("archive-start-new").addEventListener("click", () => startArchiveSession().catch(showError));
byId("archive-select-all").addEventListener("change", toggleArchiveSelection);
byId("maintenance-settings-form").addEventListener("submit", (event) => saveMaintenanceSettings(event).catch(showError));
byId("maintenance-select-all").addEventListener("change", toggleMaintenanceSelection);
byId("access-more").addEventListener("click", () => loadAccess({ append: true }).catch(showError));
byId("health-window").addEventListener("change", () => healthView.load({ reload: true }).catch(showError));
refresh();
loadEnrollment();
window.setInterval(() => loadEnrollment(), 3000);
