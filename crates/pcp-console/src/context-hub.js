// Bounded operational UI. Draft decisions are reversible until explicit submission.
export function pendingCandidate(item) { return ["pending", "deferred"].includes(item.status); }
export function reviewDraft(items, action, {title, content, targetRevisionId} = {}) {
  if (!items.length || items.some((item) => !pendingCandidate(item))) throw new Error("Candidate is not reviewable");
  if (new Set(items.map((item) => item.candidateId)).size !== items.length) throw new Error("Duplicate candidate");
  if (new Set(items.map((item) => item.input.scope)).size !== 1) throw new Error("Candidates must share a Scope");
  if (action === "promote" && (!title?.trim() || !content?.trim())) throw new Error("Reviewed title and content are required");
  if (action === "represented" && !targetRevisionId?.trim()) throw new Error("Exact existing Revision is required");
  if (!["promote", "represented", "defer", "reject"].includes(action)) throw new Error("Invalid review decision");
  return { candidates:items.map((item) => ({candidateId:item.candidateId, version:item.version})), action, title, content, targetRevisionId };
}
export function reconcileDrafts(drafts, items) {
  const before = drafts.size;
  const current = new Map(items.map((item) => [item.candidateId, item]));
  for (const [key, draft] of drafts) {
    if (draft.candidates.some((ref) => current.get(ref.candidateId)?.version !== ref.version || !pendingCandidate(current.get(ref.candidateId) || {}))) drafts.delete(key);
  }
  return before - drafts.size;
}

export function createContextHub({root, request, mutate, confirmAction, icon, language, openPage, formatTime, onCommitted = async () => {}}) {
  let snapshot = null, clients = [], tab = "candidates", busy = false, error = "", progress = "", loading = false;
  const drafts = new Map(), selected = new Set();
  let editor = null;
  const text = (zh,en) => language() === "zh" ? zh : en;
  const node = (tag, className = "", value = "") => { const n = document.createElement(tag); n.className = className; n.textContent = value; return n; };
  const operation = (op, params) => mutate("/api/context-hub", {operation:op, params});
  function button(label, action, glyph, primary = false) {
    const b = node("button", `${glyph ? "icon-button" : ""}${primary ? " primary" : ""}`, glyph ? "" : label);
    b.type = "button"; b.disabled = busy || loading;
    b.setAttribute("aria-label", label); b.title = label;
    if (glyph) b.append(icon(glyph));
    b.addEventListener("click", () => Promise.resolve().then(action).catch(fail));
    return b;
  }
  function fail(e) { error = e?.message || String(e); render(); }
  async function load() {
    if (busy || loading) return;
    loading = true; render();
    try {
      snapshot = await request("/api/context-hub");
      const stale = reconcileDrafts(drafts, snapshot.candidates);
      const ids = new Set(snapshot.candidates.filter(pendingCandidate).map((c) => c.candidateId));
      for (const id of selected) if (!ids.has(id)) selected.delete(id);
      try { clients = (await request("/api/enrollment")).result.registrations || []; } catch (_) { clients = []; }
      error = stale ? text(`${stale} 项暂存决定因候选已变化或过期而失效，请重新审阅。`, `${stale} staged decisions expired or changed; review them again.`) : "";
    } catch(e) { error = e.message || String(e); }
    finally { loading = false; render(); }
  }
  function stage(items, action, fields = {}) {
    const draft = reviewDraft(items, action, fields);
    for (const [key, previous] of drafts) if (previous.candidates.some((ref) => items.some((item) => item.candidateId === ref.candidateId))) drafts.delete(key);
    drafts.set(draft.candidates[0].candidateId, draft);
    for (const item of items) selected.delete(item.candidateId);
    editor = null; error = ""; render();
  }
  function startEditor(items, action = "promote") {
    if (new Set(items.map((i) => i.input.scope)).size !== 1) throw new Error(text("合并候选必须属于同一范围", "Merge candidates within one Scope"));
    editor = {items, action, title:items[0].input.title, content:items.map((i) => i.input.content).join("\n\n"), targetRevisionId:""};
    render();
  }
  async function apply() {
    if (!drafts.size || busy) return;
    const accepted = await confirmAction({title:text("提交这批审阅决定？","Submit these review decisions?"),
      description:text("只有接受的候选会成为正式 Page。提交后不能在此撤销；尚未提交的决定随时可撤销。", "Only promoted candidates become Pages. Unsubmitted decisions can be undone; submitted decisions cannot be undone here."),
      confirmLabel:text("提交审阅","Submit review")});
    if (!accepted) return;
    busy = true; error = "";
    let committed = false;
    const batch = [...drafts.entries()];
    try {
      for (let i=0;i<batch.length;i++) {
        const [key,draft] = batch[i]; progress = `${i+1} / ${batch.length}`; render();
        const result = await operation("review", draft);
        committed = true;
        for (const ref of draft.candidates) {
          const item = snapshot.candidates.find((c) => c.candidateId === ref.candidateId);
          if (item) { item.status = result.status; item.result = result; item.version++; }
        }
        drafts.delete(key);
      }
      progress = text("本批决定已提交", "Review submitted");
    } catch(e) { error = e.message || String(e); }
    finally {
      if (committed) {
        try { await onCommitted(); }
        catch (_) { if (!error) error = text("决定已提交，但页面统计刷新失败，可稍后刷新。", "Decisions submitted, but page counts could not refresh. Refresh later."); }
      }
      busy = false; render();
    }
  }
  function renderEditor() {
    if (!editor) return null;
    const wrap = node("section", "context-card context-editor");
    wrap.append(node("h3", "", text(`审阅 ${editor.items.length} 条候选`, `Review ${editor.items.length} candidates`)));
    for (const [field,label,multi] of editor.action === "represented"
      ? [["targetRevisionId",text("现有 Page 的精确 Revision ID","Exact existing Revision ID"),false]]
      : [["title",text("标题","Title"),false],["content",text("正式内容（请消除重复、保留不同观点）","Reviewed content (remove duplication, preserve differing claims)"),true]]) {
      const labelNode = node("label", "", label), input = node(multi ? "textarea" : "input");
      input.value = editor[field]; input.disabled = busy; input.addEventListener("input", () => { editor[field] = input.value; });
      labelNode.append(input); wrap.append(labelNode);
    }
    const actions = node("div", "context-actions");
    actions.append(button(text("暂存决定","Stage decision"), () => stage(editor.items, editor.action, editor), "accept", true),
      button(text("取消","Cancel"), () => {editor=null;render();}, "end"));
    wrap.append(actions); return wrap;
  }
  function renderCandidates() {
    const box = node("div");
    box.append(node("p", "context-note", text("候选不进入正式召回。相似只供对照，提及次数不代表事实已确认。决定暂存后会收成横条，提交前可撤销。", "Candidates are excluded from recall. Similarity suggests comparison, not confirmation. Stage decisions, undo freely, then submit.")));
    const actions = node("div", "context-actions");
    const combine = button(text(`合并审阅所选 (${selected.size})`,`Review selected together (${selected.size})`), () => {
      const items = snapshot.candidates.filter((c) => selected.has(c.candidateId));
      if (items.length) startEditor(items);
    }, "pack");
    combine.disabled = busy || loading || !selected.size;
    actions.append(combine);
    box.append(actions);
    if (editor) box.append(renderEditor());
    const consumed = new Set();
    for (const [key,draft] of drafts) {
      draft.candidates.forEach((r) => consumed.add(r.candidateId));
      const row = node("div", "context-card context-resolved");
      const subject = draft.title || snapshot.candidates.find((c) => c.candidateId === draft.candidates[0].candidateId)?.input.title || "";
      const label = node("strong", "", `${actionLabel(draft.action)} · ${subject} · ${draft.candidates.length} ${text("条 · 待提交", "items · unsubmitted")}`);
      label.title = label.textContent;
      row.append(label,
        button(text("撤销","Undo"), () => {drafts.delete(key);render();}, "undo"));
      box.append(row);
    }
    if (!snapshot.candidates.length) box.append(node("div", "context-empty", text("没有待留存候选", "No candidate memories")));
    for (const item of [...snapshot.candidates].reverse()) {
      if (consumed.has(item.candidateId)) continue;
      const card = node("article", `context-card${pendingCandidate(item) || item.status === "promoting" ? "" : " context-resolved"}`);
      const title = node("h3", "", item.input.title);
      if (!pendingCandidate(item) && item.status !== "promoting") {
        card.append(node("strong", "", `${item.input.title} · ${actionLabel(item.status)}`));
        if (item.result?.pageId) card.append(button(text("打开 Page","Open Page"), () => openPage(item.result.pageId), "open"));
        box.append(card); continue;
      }
      const heading = node("div", "context-select");
      const check = node("input"); check.type="checkbox"; check.checked=selected.has(item.candidateId); check.disabled=busy || item.status === "promoting";
      check.setAttribute("aria-label", text("选择候选","Select candidate"));
      check.addEventListener("change", () => {check.checked ? selected.add(item.candidateId) : selected.delete(item.candidateId);render();});
      heading.append(check,title); card.append(heading);
      card.append(node("p", "context-meta", `${item.clientId} · ${item.input.scope} · ${formatTime(item.createdAt)}`), node("p", "", item.input.content));
      if (item.snoozedUntil) card.append(node("p", "context-note", `${text("暂缓至","Deferred until")} ${formatTime(item.snoozedUntil)}`));
      const similar = snapshot.similarCandidates?.[item.candidateId] || [];
      if (similar.length) card.append(node("p", "context-note", text(`另有 ${similar.length} 条相似候选，可选择后合并审阅`, `${similar.length} similar candidates; select to review together`)));
      const details = node("details"); details.append(node("summary", "", text("来源与标识","Sources and identifiers")), node("p", "context-meta", JSON.stringify({candidateId:item.candidateId,eventId:item.input.eventId,sourceRefs:item.input.sourceRefs,basedOnRevisionIds:item.input.basedOnRevisionIds})));
      card.append(details);
      const actions = node("div", "context-actions");
      if (item.status === "promoting") {
        actions.append(node("span", "context-note", text("提交结果待确认，请用原决定重试", "Submission outcome unknown; retry the exact decision")), button(text("重试","Retry"), async () => {
          busy=true;render();try { await operation("review", item.promotionRequest); } finally {busy=false;} await load();
        }, "retry"));
      } else {
        actions.append(button(text("接受并编辑","Accept and edit"), () => startEditor([item]), "accept"),
          button(text("已有记录涵盖","Already represented"), () => startEditor([item], "represented"), "relation"),
          button(text("暂缓","Defer"), () => stage([item],"defer"), "defer"),
          button(text("不保留","Reject"), () => stage([item],"reject"), "archive"));
      }
      card.append(actions);box.append(card);
    }
    const footer=node("div","context-toolbar");
    footer.append(node("span","", busy ? `${text("正在提交","Submitting")} ${progress}` : `${drafts.size} ${text("项决定待提交","decisions unsubmitted")}`),button(text("提交审阅","Submit review"),apply,"apply",true));
    footer.lastChild.disabled=busy || !drafts.size;box.append(footer);return box;
  }
  function actionLabel(action) {
    return ({promote:text("收为正式 Page","Promote"),promoted:text("已收为 Page","Promoted"),represented:text("现有记录已涵盖","Already represented"),defer:text("暂缓","Defer"),deferred:text("已暂缓","Deferred"),reject:text("不保留","Reject"),rejected:text("已拒绝","Rejected")})[action] || action;
  }
  function renderActivity() {
    const box=node("div");box.append(node("p","context-note",text("可选的跨窗口近况，每客户端最多 3 个主题；不进入长期召回。没有更新不代表没有活动，过期不代表任务结束。", "Optional cross-window updates, up to 3 topics/client, excluded from durable recall. Silence is not inactivity; expiry is not completion.")));
    if (!snapshot.activity.length) box.append(node("div","context-empty",text("当前没有共享近况","No current activity cards")));
    for (const item of snapshot.activity) {
      const card=node("article","context-card");card.append(node("h3","",item.topicKey),node("p","",item.summary),node("p","context-meta",`${item.clientId} · ${item.scope} · ${text("更新","Updated")} ${formatTime(item.updatedAt)} · ${text("过期","Expires")} ${formatTime(item.expiresAt)}`));
      const actions=node("div","context-actions");actions.append(button(text("移除近况","Remove activity"),async()=>{
        if (!await confirmAction({title:text("移除这条近况？","Remove this activity card?"),description:text("不会删除任何正式 Page。","No durable Page is deleted."),confirmLabel:text("移除","Remove")})) return;
        busy=true;render();try { await operation("remove_activity",{card_id:item.cardId,version:item.version}); } finally {busy=false;} await load();
      },"delete"));card.append(actions);box.append(card);
    }return box;
  }
  function renderPolicies() {
    const box=node("div");box.append(node("p","context-note",text("三个入口分别授权，默认关闭；仍受原 Scope 权限限制。允许发布意味着该范围内已获近况读取许可的客户端可以看到内容。", "Each capability is opt-in and still Scope-bound. Publishing shares cards with activity-enabled readers of that Scope.")));
    const identities=new Map(clients.map((c)=>[c.client.principal.principalId,c.client.principal.displayName]));
    snapshot.policies.forEach((p)=>{if(!identities.has(p.clientId))identities.set(p.clientId,p.clientId);});
    // Configured non-enrollment clients can be entered explicitly by the operator.
    const form=node("form","context-card context-policy"), input=node("input");input.placeholder=text("客户端 Principal ID","Client Principal ID");input.setAttribute("aria-label",input.placeholder);input.disabled=busy;
    const add=button(text("添加客户端","Add client"),()=>{},"access");add.type="submit";form.append(input,add);
    form.addEventListener("submit",(event)=>{event.preventDefault();if(!input.value.trim()||busy)return;savePolicy({clientId:input.value.trim()}).catch(fail);});box.append(form);
    for(const [id,name] of identities){
      const p={clientId:id,submitCandidates:false,publishActivity:false,readActivity:false,...snapshot.policies.find((p)=>p.clientId===id)};
      const card=node("section","context-card");card.append(node("h3","",name||id),node("p","context-meta",id));
      const options=node("div","context-policy");
      for(const [key,label] of [["submitCandidates",text("提交候选","Submit candidates")],["publishActivity",text("发布近况","Publish activity")],["readActivity",text("读取近况","Read activity")]]){
        const labelNode=node("label","",label),check=node("input");check.type="checkbox";check.checked=p[key];check.disabled=busy;check.addEventListener("change",()=>{p[key]=check.checked;});labelNode.prepend(check);options.append(labelNode);
      }
      options.append(button(text("保存权限","Save permissions"),()=>savePolicy(p),"accept"));card.append(options);box.append(card);
    }return box;
  }
  async function savePolicy(policy){busy=true;render();try{await operation("set_policy",policy);}finally{busy=false;}await load();}
  function render(){
    root.replaceChildren();root.setAttribute("aria-busy",String(busy||loading));
    const heading=node("div","context-heading");heading.append(node("h2","",text("暂存与近况","Context inbox")),button(text("刷新","Refresh"),load,"refresh"));root.append(heading);
    if(error){const alert=node("p","context-error",error);alert.setAttribute("role","alert");root.append(alert);}
    if(loading)root.append(node("p","context-note",text("正在读取…","Loading…")));
    if(!snapshot)return;
    const tabs=node("div","context-subtabs");tabs.setAttribute("role","tablist");
    for(const [key,label] of [["candidates",text("候选记忆","Candidates")],["activity",text("当前近况","Activity")],["policies",text("客户端权限","Client permissions")]]){
      const b=button(label,()=>{tab=key;render();});b.setAttribute("role","tab");b.setAttribute("aria-selected",String(tab===key));tabs.append(b);
    }root.append(tabs);
    root.append(tab==="candidates"?renderCandidates():tab==="activity"?renderActivity():renderPolicies());
  }
  return {load,render};
}
