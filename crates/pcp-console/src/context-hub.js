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
export function candidateReviewQueue(synthesis) {
  if (["promoted", "interrupted"].includes(synthesis.status)) return "completed";
  if (["accumulating", "needs_review", "stale"].includes(synthesis.automaticReview?.state)) return "waiting";
  return "current";
}

// Receipts use submitted-output indexes, which can differ from draft indexes.
export function synthesisOutputRows(synthesis) {
  const automatic = synthesis.automaticReview, submitted = synthesis.reviewRequest?.outputs;
  if (!submitted) return synthesis.outputs.map((output, index) => ({output, assessment:automatic?.decisions?.find(d => d.outputIndex === index)}));
  const approved = (automatic?.decisions || []).filter(d => d.verdict === "approve");
  const rows = submitted.map((output, resultIndex) => ({output, resultIndex, result:synthesis.results?.[resultIndex], assessment:approved[resultIndex]}));
  for (const assessment of automatic?.decisions || []) {
    if (assessment.verdict !== "approve" && synthesis.outputs[assessment.outputIndex]) rows.push({output:synthesis.outputs[assessment.outputIndex], assessment});
  }
  return rows;
}

export function completedCandidateIds(candidates, syntheses) {
  const completed = new Set();
  for (const synthesis of syntheses) {
    if (synthesis.status !== "promoted") continue;
    const rows = synthesisOutputRows(synthesis);
    for (const ref of synthesis.candidates) {
      const candidate = candidates.find(c => c.candidateId === ref.candidateId);
      // A later submission must remain visible even when its earlier version was written.
      if (!candidate || candidate.version !== ref.version + 1) continue;
      const used = rows.filter(row => row.output.candidateIds.includes(ref.candidateId));
      if (used.length && used.every(row => row.result?.pageId)) completed.add(ref.candidateId);
    }
  }
  return completed;
}

export function automaticReviewLabel(state, english = false) {
  const labels = {
    running:["Sol 审核中", "Sol review running"], waiting_budget:["等待共享审核额度或服务恢复", "Waiting for shared budget or service"],
    approved:["审核通过，等待写入", "Approved, awaiting write"], applying:["正在写入", "Writing memories"], written:["Sol 已审核并自动写入", "Reviewed by Sol and written automatically"],
    accumulating:["继续积累证据", "Awaiting more evidence"], needs_input:["需要你补充一个具体信息", "A specific answer is needed"],
    needs_review:["自动审核暂停，候选已保留", "Automatic review held; candidates retained"], stale:["依据已变化，等待重新整理", "Evidence changed; awaiting organization"],
    write_interrupted:["写入中断，可继续原计划", "Write interrupted; exact plan can resume"]
  };
  return labels[state]?.[english ? 1 : 0] || state || "";
}

export function synthesisDraft(synthesis, items, outputs) {
  if (synthesis.status !== "pending") throw new Error("Synthesis is not reviewable");
  const current = new Map(items.map(item => [item.candidateId, item]));
  if (synthesis.candidates.some(ref => current.get(ref.candidateId)?.version !== ref.version || !pendingCandidate(current.get(ref.candidateId) || {}))) throw new Error("Synthesis evidence changed");
  if (!outputs.length || outputs.length > 4) throw new Error("Choose 1–4 memory outputs");
  const offered = new Set(synthesis.candidates.map(ref => ref.candidateId)), covered = new Set(), targets = new Set();
  for (const output of outputs) {
    if (!output.title?.trim() || !output.content?.trim()) throw new Error("Reviewed title and content are required");
    if (!output.candidateIds.length || new Set(output.candidateIds).size !== output.candidateIds.length || output.candidateIds.some(id => !offered.has(id))) throw new Error("Output evidence is invalid");
    if (output.action === "update" && !(synthesis.updateableRevisionIds || []).includes(output.targetRevisionId)) throw new Error("Target cannot be updated");
    output.candidateIds.forEach(id => covered.add(id));
    if (!["create", "update", "represented"].includes(output.action)) throw new Error("Invalid memory action");
    if (output.action === "create" ? Boolean(output.targetRevisionId) : !synthesis.offeredRevisionIds.includes(output.targetRevisionId)) throw new Error("Exact offered Revision required");
    if (output.targetRevisionId && targets.has(output.targetRevisionId)) throw new Error("Cannot target one Page twice");
    if (output.targetRevisionId) targets.add(output.targetRevisionId);
  }
  return {synthesisId:synthesis.synthesisId, version:synthesis.version, outputs:structuredClone(outputs), candidates:synthesis.candidates.filter(ref => covered.has(ref.candidateId))};
}
export function reconcileDrafts(drafts, items, syntheses = []) {
  const before = drafts.size;
  const current = new Map(items.map((item) => [item.candidateId, item]));
  for (const [key, draft] of drafts) {
    if (draft.candidates.some((ref) => current.get(ref.candidateId)?.version !== ref.version || !pendingCandidate(current.get(ref.candidateId) || {})) || (draft.synthesisId && !syntheses.some(s => s.synthesisId === draft.synthesisId && s.version === draft.version && s.status === "pending"))) drafts.delete(key);
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
      const stale = reconcileDrafts(drafts, snapshot.candidates, snapshot.syntheses || []);
      const completed = completedCandidateIds(snapshot.candidates, snapshot.syntheses || []);
      const ids = new Set(snapshot.candidates.filter(c => pendingCandidate(c) && !completed.has(c.candidateId)).map((c) => c.candidateId));
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
        const {candidates, ...synthesisParams} = draft;
        const result = await operation(draft.synthesisId ? "review_synthesis" : "review", draft.synthesisId ? synthesisParams : draft);
        committed = true;
        for (const ref of draft.candidates) {
          const item = snapshot.candidates.find((c) => c.candidateId === ref.candidateId);
          if (item) { item.status = result.status; item.result = result; item.version++; }
        }
        if (draft.synthesisId) {
          const synthesis = snapshot.syntheses.find(s => s.synthesisId === draft.synthesisId);
          if (synthesis) { synthesis.status = result.status; synthesis.results = result.outputs; }
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
      busy = false;
      if (error) { const failure = error; await load(); error = failure; }
      render();
    }
  }
  function renderEditor() {
    if (!editor) return null;
    if (editor.synthesis) return renderSynthesisEditor();
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
    box.append(node("p", "context-note", text("后台会把相关候选整理为连续的记忆草案，保留演变、分歧和来源。成熟草案可由 Sol 审核写入；你也可以编辑或手动审阅。未解决的候选继续积累，不要求逐条处理。", "Background organization connects related evidence while preserving its evolution, disagreements and sources. Sol can review and write mature drafts; you can also edit or review them manually. Unresolved candidates continue accumulating without requiring individual action.")));
    box.append(renderOrganization());
    const actions = node("div", "context-actions");
    const combine = button(text(`合并审阅所选 (${selected.size})`,`Review selected together (${selected.size})`), () => {
      const items = snapshot.candidates.filter((c) => selected.has(c.candidateId));
      if (items.length) startEditor(items);
    }, "pack");
    combine.disabled = busy || loading || !selected.size;
    actions.append(combine);
    box.append(actions);
    if (editor) box.append(renderEditor());
    const consumed = completedCandidateIds(snapshot.candidates, snapshot.syntheses || []);
    const waiting = node("details", "context-waiting"), completed = node("details", "context-completed");
    waiting.append(node("summary", "", text("继续积累 / 暂缓事项", "Accumulating / deferred items")));
    completed.append(node("summary", "", text("已完成的审阅记录", "Completed review history")));
    for (const [key,draft] of drafts) {
      draft.candidates.forEach((r) => consumed.add(r.candidateId));
      const row = node("div", "context-card context-resolved");
      const subject = draft.outputs?.map(o => o.title).join(" · ") || draft.title || snapshot.candidates.find((c) => c.candidateId === draft.candidates[0].candidateId)?.input.title || "";
      const label = node("strong", "", `${draft.synthesisId ? text("整理草案", "Synthesis") : actionLabel(draft.action)} · ${subject} · ${draft.candidates.length} ${text("条 · 待提交", "items · unsubmitted")}`);
      label.title = label.textContent;
      row.append(label,
        button(text("撤销","Undo"), () => {drafts.delete(key);render();}, "undo"));
      box.append(row);
    }
    for (const synthesis of snapshot.syntheses || []) {
      if (!["pending", "promoting", "promoted", "interrupted"].includes(synthesis.status)) continue;
      if (!["promoted", "interrupted"].includes(synthesis.status) && synthesis.candidates.some(ref => consumed.has(ref.candidateId))) continue;
      if (synthesis.status === "pending" && synthesis.candidates.some(ref => !snapshot.candidates.some(c => c.candidateId === ref.candidateId && c.version === ref.version && pendingCandidate(c)))) continue;
      const lane = candidateReviewQueue(synthesis);
      (lane === "completed" ? completed : lane === "waiting" ? waiting : box).append(renderSynthesis(synthesis));
      synthesis.candidates.forEach(ref => { if (!["promoted", "interrupted"].includes(synthesis.status) || !pendingCandidate(snapshot.candidates.find(c => c.candidateId === ref.candidateId) || {})) consumed.add(ref.candidateId); });
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
      card.append(actions);(item.result?.status === "partially_represented" ? waiting : box).append(card);
    }
    if (waiting.children.length > 1) { waiting.firstChild.textContent += ` · ${waiting.children.length - 1}`; box.append(waiting); }
    if (completed.children.length > 1) { completed.firstChild.textContent += ` · ${completed.children.length - 1}`; box.append(completed); }
    const footer=node("div","context-toolbar");
    footer.append(node("span","", busy ? `${text("正在提交","Submitting")} ${progress}` : `${drafts.size} ${text("项决定待提交","decisions unsubmitted")}`),button(text("提交审阅","Submit review"),apply,"apply",true));
    footer.lastChild.disabled=busy || !drafts.size;box.append(footer);return box;
  }
  function renderOrganization() {
    const box = node("div", "context-organization"), state = snapshot.organization || {};
    const status = !state.available ? text("后台整理尚未启用；需要开启 Runtime 自动维护。", "Background organization needs enabled Runtime maintenance.")
      : state.queued ? text("已排队，将在下一次后台检查时整理。", "Queued for the next background check.")
      : state.error && state.retryWhenChanged ? text("上轮整理未通过校验，原始候选已保留。新增依据或手动重新整理后再尝试，避免重复消耗。", "The previous draft failed validation. Originals remain; new evidence or an explicit retry can resume organization without repeated model calls.")
      : state.error ? text("整理暂未完成，原始候选已保留，后台会延后重试。", "Organization could not complete. Originals are retained; background work will retry later.")
      : text("新内容会在约 2 分钟的合并等待后整理；没有新证据时不重复调用模型。", "New evidence is organized after about two minutes of coalescing. Unchanged evidence does not trigger repeated model calls.");
    box.append(node("p", "context-note", status));
    if (state.lastCompletedAt) box.append(node("p", "context-meta", `${text("上次整理", "Last organized")} ${formatTime(state.lastCompletedAt)}`));
    if (state.error) { const details = node("details"); details.append(node("summary", "", text("查看原因", "Details")), node("p", "context-error", state.error)); box.append(details); }
    const queue = button(text("重新整理候选", "Organize candidates"), async () => {
      busy = true; render(); try { await operation("organize_candidates"); } finally { busy = false; } await load();
    });
    queue.disabled = busy || loading || !state.available || !snapshot.candidates.some(pendingCandidate);
    box.append(queue);
    const automatic = state.automaticReviewEnabled === true;
    box.append(node("p", "context-note", state.automaticReviewAvailable
      ? text("草案稳定约 5 分钟后进入 Sol 共享审核队列。逐条通过后自动写入，未解决的部分继续保留；额度不足时等待。", "After drafts settle for about five minutes, Sol reviews them within the shared budget. Approved outputs are written; unresolved evidence remains. Budget shortages wait.")
      : text("自动写入需要启用维护写入模式和共享高级审核额度。", "Automatic writes require maintenance apply mode and the shared upgraded-review budget.")));
    const toggle = button(automatic ? text("暂停自动审核写入", "Pause automatic review and writes") : text("启用自动审核写入", "Enable automatic review and writes"), async () => {
      busy = true; render(); try { await operation("set_automatic_review", {enabled:!automatic}); } finally { busy = false; } await load();
    });
    toggle.disabled = busy || loading; box.append(toggle); return box;
  }
  function renderSynthesis(synthesis) {
    const card = node("article", "context-card context-synthesis");
    card.append(node("h3", "", synthesis.title));
    const terminal = ["promoted", "interrupted"].includes(synthesis.status), automatic = synthesis.automaticReview;
    if (automatic) {
      card.append(node("p", "context-auto-review", automaticReviewLabel(automatic.state, language() !== "zh")));
      const audit = node("details", "context-review-audit"); audit.append(node("summary", "", text("高级审核记录", "Upgraded review record")));
      if (automatic.reason) audit.append(node("p", "context-note", automatic.reason));
      for (const step of automatic.steps || []) audit.append(node("p", "context-meta", `${step.tier} · ${step.stage} · ${step.state}${step.requestId ? ` · ${step.requestId}` : ""}${step.actualTokens != null ? ` · ${step.actualTokens} tokens` : ""}${step.reason ? ` · ${step.reason}` : ""}`));
      if (automatic.input?.pages?.length) {
        const compared = node("details"); compared.append(node("summary", "", text(`审核时对照的已有记忆 · ${automatic.input.pages.length}`, `Memories compared during review · ${automatic.input.pages.length}`)));
        for (const page of automatic.input.pages) {
          const source = node("details"); source.append(node("summary", "", page.facets?.title || page.content?.split("\n")[0]?.replace(/^#+\s*/, "").slice(0,160) || page.pageId), node("p", "context-meta", page.revisionId), node("p", "", page.content || ""), button(text("打开当前记忆", "Open current memory"), () => openPage(page.pageId))); compared.append(source);
        }
        audit.append(compared);
      }
      card.append(audit);
    }
    if (terminal) {
      card.append(node("p", "context-note", synthesis.status === "interrupted" ? text("已停止继续提交。已确认结果保留，其余候选重新整理；请对照已有页面确认未收到回执的输出。", "Submission stopped. Confirmed results remain; evidence returns to organization. Check existing Pages for any output whose receipt was not recorded.") : text("已完成审阅", "Review completed")));
    }
    if (!terminal) card.append(node("p", "context-maturity", synthesis.maturity === "ready" ? text("已形成可审阅的记忆", "Ready for memory review") : text("持续积累中", "Still developing")));
    // Original evidence precedes the model's interpretation.
    const timeline = node("details", "context-timeline");
    timeline.append(node("summary", "", text(`${synthesis.candidates.length} 条原始候选 · 查看演变`, `${synthesis.candidates.length} source candidates · inspect evolution`)));
    for (const ref of synthesis.candidates) {
      const item = snapshot.candidates.find(c => c.candidateId === ref.candidateId);
      if (!item) continue;
      const source = node("section"); source.append(node("h4", "", item.input.title), node("p", "context-meta", `${formatTime(item.createdAt)} · ${item.clientId}`), node("p", "", item.input.content));
      const provenance = node("details"); provenance.append(node("summary", "", text("来源", "Sources")), node("p", "context-meta", JSON.stringify({sourceRefs:item.input.sourceRefs, basedOnRevisionIds:item.input.basedOnRevisionIds})));
      source.append(provenance); timeline.append(source);
    }
    card.append(timeline, node("h4", "", text("整理后的演变", "Interpreted evolution")), node("p", "", synthesis.narrative), node("p", "context-note", synthesis.reason));
    if (synthesis.unresolved.length) {
      card.append(node("h4", "", text("仍未解决", "Unresolved")));
      const list = node("ul"); synthesis.unresolved.forEach(question => list.append(node("li", "", question))); card.append(list);
    }
    for (const [i, {output, assessment, result, resultIndex}] of synthesisOutputRows(synthesis).entries()) {
      const memory = node("section", "context-memory-output"), shown = resultIndex == null ? assessment?.revision || output : output;
      memory.append(node("h4", "", `${i + 1}. ${shown.title}`), node("p", "context-note", memoryActionLabel(output.action)), node("p", "", shown.content));
      if (assessment) memory.append(node("p", "context-note", `${({approve:text("审核通过", "Approved"),accumulating:text("继续积累", "Accumulating"),needs_input:text("需要补充信息", "Input needed"),no_change:text("无需写入", "No write needed")})[assessment.verdict] || assessment.verdict} · ${assessment.reason}`));
      const target = (snapshot.synthesisContexts?.[synthesis.contextId] || synthesis.comparedPages || []).find(page => page.revisionId === output.targetRevisionId);
      if (target) {
        const comparison = node("details"); comparison.append(node("summary", "", text("对照现有记忆", "Compare existing memory")), node("p", "", target.content || target.summary || ""), button(text("打开现有记忆", "Open existing memory"), () => openPage(target.pageId))); memory.append(comparison);
      }
      if (result?.pageId) {
        const row = node("div", "context-actions");
        row.append(button(text("打开记忆 / 纠正", "Open / correct memory"), () => openPage(result.pageId)));
        const undone = automatic?.undoResults?.find(r => r.outputIndex === resultIndex && r.status !== "withdrawing");
        if (undone) row.append(node("span", "context-note", text("已撤回，历史内容保留", "Withdrawn; history retained")));
        else if (automatic?.authorized && synthesis.status === "promoted" && ["create", "update"].includes(output?.action)) {
          const undo = button(text("撤回这次自动写入", "Withdraw this automatic write"), async () => {
            const accepted = await confirmAction({title:text("撤回自动写入", "Withdraw automatic write"),description:output.action === "create" ? text("这条记忆将归档保留，不再参与默认检索。", "This memory will be archived and excluded from default retrieval.") : text("恢复自动更新前的正文，保留修订历史；如果此后又有修改，将拒绝覆盖。", "Restore the content before this automatic update, preserving history. Later changes will not be overwritten."),confirmLabel:text("撤回", "Withdraw")});
            if (!accepted) return;
            busy = true; render(); try { await operation("undo_automatic_output", {synthesis_id:synthesis.synthesisId,version:synthesis.version,output_index:resultIndex}); await onCommitted(); } finally { busy = false; } await load();
          });
          undo.disabled = busy || loading; row.append(undo);
        }
        memory.append(row);
      }
      card.append(memory);
    }
    if (terminal) return card;
    const actions = node("div", "context-actions");
    if (synthesis.status === "promoting") {
      actions.append(node("span", "context-note", text(`已完成 ${synthesis.results.length} 项输出；继续原审阅决定`, `${synthesis.results.length} outputs committed; resume the exact review`)), button(text("继续提交", "Resume submission"), async () => {
        busy = true; render(); try { await operation("review_synthesis", synthesis.reviewRequest); await onCommitted(); } finally { busy = false; } await load();
      }));
      actions.append(button(text("停止并重新整理", "Stop and reorganize"), async () => {
        if (!await confirmAction({title:text("停止继续这份审阅决定？", "Stop this submission?"), description:text("已经写入的记忆不会撤销。未确认的候选将重新整理；请先核对已有页面，避免重复保存结果未知的输出。", "Committed memories are retained. Unconfirmed evidence returns to organization. Check existing Pages first to avoid duplicating an output with an unknown result."), confirmLabel:text("停止并保留结果", "Stop and retain results")})) return;
        busy = true; render(); try { await operation("stop_synthesis", {synthesisId:synthesis.synthesisId,version:synthesis.version}); } finally {busy = false;} await load();
      }));
    } else {
      actions.append(button(text("编辑记忆草案", "Edit memory drafts"), () => {
        editor = {synthesis, outputs:structuredClone(synthesis.outputs.length ? synthesis.outputs : [{candidateIds:synthesis.candidates.map(c => c.candidateId),title:synthesis.title,content:synthesis.narrative,action:"create",targetRevisionId:null}])}; render();
      }, null, true));
      const items = snapshot.candidates.filter(c => synthesis.candidates.some(ref => ref.candidateId === c.candidateId));
      actions.append(button(text("继续积累", "Keep developing"), () => stage(items, "defer")), button(text("不保留这组", "Reject group"), () => stage(items, "reject")));
    }
    card.append(actions); return card;
  }
  function memoryActionLabel(action) { return ({create:text("新增记忆", "New memory"),update:text("更新现有记忆", "Update existing memory"),represented:text("现有记忆已涵盖", "Already represented")})[action] || action; }
  function renderSynthesisEditor() {
    const wrap = node("section", "context-card context-editor"), synthesis = editor.synthesis;
    wrap.append(node("h3", "", text("编辑整理结果", "Edit synthesis outputs")), node("p", "context-note", text("每条输出分别选择依据；未被输出采用的候选会继续保留。更新现有记忆时，请保留原有的独立信息。", "Choose evidence for each output. Unused candidates remain available. Preserve independent information when updating an existing memory.")));
    editor.outputs.forEach((output, index) => {
      const part = node("section", "context-memory-output");
      for (const [field,label,multiline] of [["title",text("标题", "Title"),false],["content",text("正式内容", "Memory content"),true]]) {
        const labelNode = node("label", "", label), input = node(multiline ? "textarea" : "input"); input.value = output[field]; input.disabled = busy; input.addEventListener("input", () => {output[field] = input.value;}); labelNode.append(input); part.append(labelNode);
      }
      const actionLabel = node("label", "", text("如何写入", "Memory action")), select = node("select");
      for (const action of ["create", "update", "represented"]) { const option = node("option", "", memoryActionLabel(action)); option.value = action; option.selected = action === output.action; option.disabled = action === "update" && !synthesis.updateableRevisionIds?.length; select.append(option); }
      select.disabled = busy; select.addEventListener("change", () => { output.action = select.value; output.targetRevisionId = output.action === "create" ? null : (output.action === "update" ? synthesis.updateableRevisionIds : synthesis.offeredRevisionIds)?.[0] || null; render(); }); actionLabel.append(select); part.append(actionLabel);
      if (output.action !== "create") {
        const label = node("label", "", text("对照的现有版本", "Compared existing Revision")), targets = node("select");
        targets.append(node("option", "", text("选择现有版本", "Select a Revision"))); targets.firstChild.value = "";
        for (const id of (output.action === "update" ? synthesis.updateableRevisionIds || [] : synthesis.offeredRevisionIds)) { const compared = (snapshot.synthesisContexts?.[synthesis.contextId] || []).find(page => page.revisionId === id); const option = node("option", "", compared?.facets?.title || compared?.content?.split("\n")[0]?.replace(/^#+\s*/, "") || id); option.value = id; option.selected = id === output.targetRevisionId; targets.append(option); }
        targets.disabled = busy; targets.addEventListener("change", () => {output.targetRevisionId = targets.value || null; render();}); label.append(targets); part.append(label);
        const compared = (snapshot.synthesisContexts?.[synthesis.contextId] || []).find(page => page.revisionId === output.targetRevisionId);
        if (compared) { const details = node("details"); details.append(node("summary", "", text("对照现有内容", "Compare existing content")), node("p", "", compared.content || compared.summary || "")); part.append(details); }
      }
      const evidence = node("fieldset"); evidence.append(node("legend", "", text("这条记忆采用的依据", "Evidence for this memory")));
      for (const ref of synthesis.candidates) {
        const label = node("label", "context-evidence-option"), check = node("input"), item = snapshot.candidates.find(c => c.candidateId === ref.candidateId);
        check.type = "checkbox"; check.checked = output.candidateIds.includes(ref.candidateId); check.disabled = busy;
        check.addEventListener("change", () => {output.candidateIds = check.checked ? [...output.candidateIds, ref.candidateId] : output.candidateIds.filter(id => id !== ref.candidateId);});
        label.append(check, node("span", "", item?.input.title || ref.candidateId)); evidence.append(label);
      }
      part.append(evidence, button(text("移除此输出", "Remove output"), () => {editor.outputs.splice(index, 1); render();})); wrap.append(part);
    });
    const actions = node("div", "context-actions"), add = button(text("增加一条记忆", "Add a memory"), () => {editor.outputs.push({candidateIds:synthesis.candidates.map(c => c.candidateId),title:"",content:"",action:"create",targetRevisionId:null}); render();});
    add.disabled = busy || editor.outputs.length >= 4;
    actions.append(add, button(text("暂存整理决定", "Stage synthesis review"), () => {
      const draft = synthesisDraft(synthesis, snapshot.candidates, editor.outputs);
      for (const [key, previous] of drafts) if (previous.candidates.some(ref => draft.candidates.some(c => c.candidateId === ref.candidateId))) drafts.delete(key);
      drafts.set(synthesis.synthesisId, draft); editor = null; render();
    }, null, true), button(text("取消", "Cancel"), () => {editor = null; render();}));
    wrap.append(actions); return wrap;
  }
  function actionLabel(action) {
    return ({promote:text("收为正式 Page","Promote"),promoted:text("已收为 Page","Promoted"),represented:text("现有记录已涵盖","Already represented"),defer:text("暂缓","Defer"),deferred:text("已暂缓","Deferred"),reject:text("不保留","Reject"),rejected:text("已拒绝","Rejected")})[action] || action;
  }
  function renderActivity() {
    const box=node("div");box.append(node("p","context-note",text("可选的跨窗口近况，每客户端最多 12 个主题；不进入长期召回。没有更新不代表没有活动，过期不代表任务结束。", "Optional cross-window updates, up to 12 topics/client, excluded from durable recall. Silence is not inactivity; expiry is not completion.")));
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
