import { technicalDetails, scopeLabel } from "./console-presentation.js";

// Legacy cards have no display title. Keep their full summary in the card body.
export function activityTitle(item) {
  if (item.title?.trim()) return item.title;
  const line = (item.summary || "").split(/\r?\n/).find(value => value.trim())?.trim().replace(/^#+\s*/, "");
  if (line) {
    const sentence = line.match(/^.*?[。！？!?](?=\s|$|[^\x00-\x7f])/)?.[0]?.trim() || line;
    const characters = [...sentence];
    return characters.length > 80 ? `${characters.slice(0, 80).join("")}…` : sentence;
  }
  return (item.topicKey || "").split(":").at(-1).replace(/[-_]+/g, " ");
}

const timestamp = item => Date.parse(item.updatedAt) || 0;
const newestFirst = (a, b) => b.updatedAt - a.updatedAt || a.item.cardId.localeCompare(b.item.cardId);

// Parent references never cross a client or Scope. Invalid/cyclic references stay visible as roots.
export function activityGroups(items, scope = "") {
  const projects = new Map();
  for (const item of items) {
    if (scope && item.scope !== scope) continue;
    if (!projects.has(item.scope)) projects.set(item.scope, []);
    projects.get(item.scope).push({item, children:[], updatedAt:timestamp(item)});
  }
  return [...projects].map(([scope, records]) => {
    const byTopic = new Map(records.map(record => [`${record.item.clientId}\0${record.item.topicKey}`, record]));
    const parents = new Map(records.map(record => [record, record.item.parentTopicKey ? byTopic.get(`${record.item.clientId}\0${record.item.parentTopicKey}`) : null]));
    const roots = [];
    for (const record of records) {
      let ancestor = parents.get(record);
      const visited = new Set([record]);
      while (ancestor && !visited.has(ancestor)) { visited.add(ancestor); ancestor = parents.get(ancestor); }
      const parent = ancestor ? null : parents.get(record);
      if (parent) parent.children.push(record); else roots.push(record);
    }
    const order = record => {
      record.children.forEach(order);
      record.updatedAt = Math.max(record.updatedAt, ...record.children.map(child => child.updatedAt));
      record.children.sort(newestFirst);
    };
    roots.forEach(order); roots.sort(newestFirst);
    return {scope, roots, count:records.length, updatedAt:Math.max(...records.map(record => record.updatedAt))};
  }).sort((a, b) => b.updatedAt - a.updatedAt || a.scope.localeCompare(b.scope));
}

export function activityStatusLabel(status, english = false) {
  const labels = {
    discussing:["讨论中", "Discussing"], in_progress:["进行中", "In progress"],
    waiting_user:["待用户", "Waiting for you"], waiting_dependency:["待集成 / 依赖", "Awaiting integration / dependency"],
    completed:["已完成", "Completed"],
  };
  return labels[status]?.[english ? 1 : 0] || status || "";
}

export function createActivityView({node, button, text, language, scopeName, formatTime, onRemove, rerender}) {
  let projectFilter = "";
  const waitingCount = records => records.reduce((count, record) => count + Number(["waiting_user", "waiting_dependency"].includes(record.item.status)) + waitingCount(record.children), 0);
  function renderCard(record) {
    const item = record.item, attention = ["waiting_user", "waiting_dependency"].includes(item.status);
    const card = node("article", `context-card context-activity-card${attention ? " context-activity-attention" : ""}`);
    const heading = node("div", "context-activity-heading");
    heading.append(node("h3", "", activityTitle(item)));
    if (item.status) heading.append(node("span", `context-activity-status${attention ? " needs-attention" : ""}`, activityStatusLabel(item.status, language() !== "zh")));
    card.append(heading);
    if (item.nextStep?.trim()) {
      const next = node("section", "context-activity-next");
      next.append(node("strong", "", text("下一步", "Next step")), node("p", "", item.nextStep));
      card.append(next);
    }
    card.append(node("p", "context-activity-summary", item.summary), node("p", "context-meta", `${text("更新", "Updated")} ${formatTime(item.updatedAt)}`));
    card.append(technicalDetails(node, text("来源与标识", "Sources and identifiers"), [
      [text("主题标识", "Topic key"), item.topicKey], [text("主主题", "Parent topic"), item.parentTopicKey],
      [text("客户端", "Client"), item.clientId], [text("范围标识", "Scope identifier"), item.scope],
      [text("过期", "Expires"), formatTime(item.expiresAt)], [text("近况标识", "Card ID"), item.cardId], [text("版本", "Version"), item.version],
    ]));
    const actions = node("div", "context-actions");
    actions.append(button(text("移除近况", "Remove activity"), () => onRemove(item), "delete")); card.append(actions);
    if (record.children.length) {
      const children = node("details", "context-activity-children");
      children.dataset.disclosure = `activity-children:${item.cardId}`;
      const waiting = waitingCount(record.children), summary = node("summary", "", text(`子主题 · ${record.children.length}`, `Subtopics · ${record.children.length}`));
      summary.dataset.focusKey = `activity-children:${item.cardId}`;
      if (waiting) summary.append(node("span", "context-activity-child-attention", text(` · ${waiting} 项待用户 / 集成`, ` · ${waiting} awaiting you / integration`)));
      children.open = waiting > 0;
      children.append(summary);
      for (const child of record.children) children.append(renderCard(child));
      card.append(children);
    }
    return card;
  }
  return {render(items, limits = {}) {
    const box = node("div", "context-activity");
    box.append(node("p", "context-note", text(
      "跨窗口共享的临时近况。状态和下一步由发布者声明；没有更新不代表没有活动，过期不代表任务结束。",
      "Temporary updates shared across windows. Publishers declare status and next steps. Silence is not inactivity; expiry is not completion.",
    )));
    const scopes = [...new Set(items.map(item => item.scope))];
    if (projectFilter && !scopes.includes(projectFilter)) projectFilter = "";
    const filters = node("div", "context-activity-filters"), label = node("label", "", text("项目 / 范围", "Project / Scope")), select = node("select");
    select.dataset.focusKey = "activity-project";
    select.setAttribute("aria-label", text("筛选项目或范围", "Filter project or Scope"));
    for (const [value, title] of [["", text("全部项目与范围", "All projects and Scopes")], ...scopes.sort((a, b) => scopeName(a).localeCompare(scopeName(b))).map(scope => [scope, scopeName(scope)])]) {
      const option = value ? scopeLabel(node, [value], scopeName, "option") : node("option", "", title); option.value = value; option.selected = value === projectFilter; select.append(option);
    }
    select.value = projectFilter; select.addEventListener("change", () => {projectFilter = select.value; rerender();});
    label.append(select); filters.append(label);
    const visible = items.filter(item => !projectFilter || item.scope === projectFilter);
    filters.append(node("span", "context-note", text(`${visible.length} 个主题 · 最近更新优先`, `${visible.length} topics · latest update first`)));
    box.append(filters);
    if (!visible.length) box.append(node("div", "context-empty", text("当前没有共享近况", "No current activity cards")));
    for (const project of activityGroups(items, projectFilter)) {
      const section = node("section", "context-activity-project"), heading = node("h3", "context-activity-project-title");
      heading.append(scopeLabel(node, [project.scope], scopeName), node("span", "context-note", ` · ${project.count}`)); section.append(heading);
      for (const record of project.roots) section.append(renderCard(record));
      box.append(section);
    }
    if (limits.topicsPerClientScope) box.append(node("p", "context-note", text(
      `每客户端在每个范围最多保留 ${limits.topicsPerClientScope} 个主题；近况不进入长期召回。`,
      `Each client can retain ${limits.topicsPerClientScope} topics per Scope. Activity is excluded from durable recall.`,
    )));
    return box;
  }};
}
