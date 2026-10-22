import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { pendingCandidate, candidateUsesIntakeSlot, reviewDraft, reconcileDrafts, candidateReviewQueue } from "../src/context-hub.js";
const candidate = (id, scope = "a") => ({candidateId:id,version:1,status:"pending",input:{scope,title:"Title",content:"Evidence"}});

test("review stages exact versions, can undo, and requires an explicit edited body", () => {
  const item=candidate("one"), draft=reviewDraft([item],"promote",{title:"Reviewed",content:"Exact final fact"});
  assert.deepEqual(draft.candidates,[{candidateId:"one",version:1}]);
  assert.equal(item.status,"pending");
  const drafts=new Map([["one",draft]]);drafts.delete("one");assert.equal(drafts.size,0);
  assert.throws(()=>reviewDraft([item],"promote"),/Reviewed/);
  assert.throws(()=>reviewDraft([item],"represented"),/Revision/);
});
test("combining different scopes, duplicates and already decided items is rejected", () => {
  assert.throws(()=>reviewDraft([candidate("a"),candidate("b","private")],"reject"),/Scope/);
  assert.throws(()=>reviewDraft([candidate("a"),candidate("a")],"reject"),/Duplicate/);
  assert.throws(()=>reviewDraft([{...candidate("a"),status:"promoted"}],"reject"),/reviewable/);
  assert.equal(pendingCandidate({...candidate("a"),status:"deferred"}),true);
  assert.equal(pendingCandidate({...candidate("a"),status:"promoting"}),false);
});
test("organized unresolved evidence does not use candidate intake slots", () => {
  const fresh = candidate("fresh");
  assert.equal(candidateUsesIntakeSlot(fresh), true);
  assert.equal(candidateUsesIntakeSlot({...fresh, organizedVersion: 1, result: {status: "partially_represented"}}), false);
  assert.equal(candidateUsesIntakeSlot({...fresh, organizedVersion: 1, status: "promoting"}), true);
});
test("reloaded or expired candidates invalidate stale decisions without applying them", () => {
  const drafts=new Map([["a",reviewDraft([candidate("a")],"reject")],["b",reviewDraft([candidate("b")],"defer")]]);
  assert.equal(reconcileDrafts(drafts,[{...candidate("a"),version:2},candidate("b")]),1);
  assert.equal(drafts.has("a"),false);assert.equal(drafts.has("b"),true);
  assert.equal(reconcileDrafts(drafts,[]),1);assert.equal(drafts.size,0);
});

import { synthesisDraft } from "../src/context-hub.js";
import { completedCandidateIds, synthesisOutputRows } from "../src/context-hub.js";
const synthesis = () => ({synthesisId:"s1", version:1, status:"pending", candidates:[{candidateId:"a",version:1},{candidateId:"b",version:1}], offeredRevisionIds:["rev1"], updateableRevisionIds:["rev1"]});
const memory = (ids = ["a", "b"]) => ({candidateIds:ids, title:"Decision", content:"Preserved decision and rationale", action:"create", targetRevisionId:null});
test("one synthesis supports multiple outputs and retains unused evidence", () => {
  const outputs = [memory(["a"]), {...memory(["a"]),title:"Independent failed attempt"}];
  const draft = synthesisDraft(synthesis(), [candidate("a"),candidate("b")], outputs);
  assert.equal(draft.outputs.length, 2);
  assert.deepEqual(draft.candidates, [{candidateId:"a",version:1}]);
  outputs[0].title="Changed after staging";
  assert.equal(draft.outputs[0].title,"Decision");
});
test("synthesis review guards exact evidence and target versions", () => {
  assert.throws(() => synthesisDraft(synthesis(),[candidate("a"),{...candidate("b"),version:2}],[memory()]),/changed/);
  assert.throws(() => synthesisDraft(synthesis(),[candidate("a"),candidate("b")],[memory(["unknown"])]),/evidence/);
  assert.throws(() => synthesisDraft(synthesis(),[candidate("a"),candidate("b")],[{...memory(),action:"update",targetRevisionId:"old"}]),/updated/);
  const update={...memory(),action:"update",targetRevisionId:"rev1"};
  assert.throws(() => synthesisDraft(synthesis(),[candidate("a"),candidate("b")],[update,update]),/twice/);
  assert.equal(synthesisDraft(synthesis(),[candidate("a"),candidate("b")],[update]).outputs[0].action,"update");
});
test("new synthesis generation invalidates a staged decision even with unchanged candidate versions", () => {
  const items=[candidate("a"),candidate("b")], draft=synthesisDraft(synthesis(),items,[memory()]);
  const drafts=new Map([["s1",draft]]);
  assert.equal(reconcileDrafts(drafts,items,[{...synthesis(),version:2}]),1);
  assert.equal(drafts.size,0);
});


test("automatic waiting and completed reviews do not become mandatory human work", () => {
  for (const state of ["waiting_budget", "running", "approved"]) assert.equal(candidateReviewQueue({status:"pending", automaticReview:{state}}), "current");
  for (const state of ["accumulating", "needs_review", "stale"]) assert.equal(candidateReviewQueue({status:"pending", automaticReview:{state}}), "waiting");
  assert.equal(candidateReviewQueue({status:"pending", automaticReview:{state:"needs_input"}}), "current");
  assert.equal(candidateReviewQueue({status:"promoted", automaticReview:{state:"written"}}), "completed");
});

test("written evidence leaves accumulation while partial and newer evidence remains", () => {
  const group = {...synthesis(), status:"promoted", outputs:[memory(["a"]),memory(["b"])], unresolved:["A future question"],
    reviewRequest:{outputs:[memory(["a"])]}, results:[{pageId:"page-a"}],
    automaticReview:{decisions:[{outputIndex:0,verdict:"approve"},{outputIndex:1,verdict:"accumulating"}]}};
  const items = [{...candidate("a"),version:2},{...candidate("b"),version:1}];
  const before = structuredClone({group,items});
  assert.deepEqual([...completedCandidateIds(items,[group])],["a"]);
  assert.deepEqual({group,items},before, "history and source evidence are not deleted or mutated");
  assert.equal(completedCandidateIds([{...items[0],version:3}],[group]).size,0);
  group.outputs[1].candidateIds.push("a");
  assert.equal(completedCandidateIds(items,[group]).size,0, "a shared source with an unapproved output stays visible");
  assert.equal(completedCandidateIds(items,[{...group,status:"interrupted"}]).size,0);
});

test("per-memory actions follow receipt order even when only a later draft was approved", () => {
  const group = {...synthesis(), outputs:[memory(["a"]),memory(["b"])],
    automaticReview:{decisions:[{outputIndex:1,verdict:"approve"},{outputIndex:0,verdict:"needs_input"}]},
    reviewRequest:{outputs:[{...memory(["b"]),title:"Actual written title",content:"Actual final content"}]},
    results:[{pageId:"page-b"}]};
  const rows = synthesisOutputRows(group);
  assert.equal(rows[0].output.title,"Actual written title");
  assert.equal(rows[0].output.content,"Actual final content");
  assert.equal(rows[0].result.pageId,"page-b");
  assert.equal(rows[0].resultIndex,0, "withdrawal uses the submitted output index");
  assert.equal(rows[0].assessment.outputIndex,1);
  assert.equal(rows[1].result,undefined, "unwritten output has no page or withdrawal action");
  assert.equal(rows[1].assessment.verdict,"needs_input");
  assert.equal(completedCandidateIds([{...candidate("b"),version:2}],[{...group,status:"promoted",results:[]}]).size,0);
});

import { createContextHub, candidateQueue } from "../src/context-hub.js";

// Small DOM adapter: exercise the real view's events and async refresh without a server.
function hubFixture(options = {}) {
  class Node {
    constructor(tag) { this.tagName=tag.toUpperCase();this.children=[];this.dataset={};this.attributes={};this.listeners={};this._text=""; }
    set textContent(value){this._text=String(value);this.children=[];}
    get textContent(){return this._text+this.children.map(c=>c.textContent).join("");}
    append(...children){this.children.push(...children);}
    prepend(...children){this.children.unshift(...children);}
    replaceChildren(...children){this.children=children;this._text="";}
    setAttribute(key,value){this.attributes[key]=value;}
    addEventListener(key,handler){this.listeners[key]=handler;}
    async fire(key, fields = {}){await this.listeners[key]?.({preventDefault(){}, ...fields});}
    get firstChild(){return this.children[0];}
    get lastChild(){return this.children.at(-1);}
    contains(target){return this===target||this.children.some(c=>c.contains(target));}
    focus(){document.activeElement=this;}
    scrollIntoView(){}
    querySelectorAll(selector){
      const match=n=>selector==="[data-focus-key]"?Boolean(n.dataset.focusKey):selector.startsWith("details")?n.tagName==="DETAILS"&&n.dataset.disclosure&&(!selector.includes("[open]")||n.open):selector.startsWith("#")?n.id===selector.slice(1):false;
      return this.children.flatMap(n=>[...(match(n)?[n]:[]),...n.querySelectorAll(selector)]);
    }
    querySelector(selector){return this.querySelectorAll(selector)[0];}
    all(){return [this,...this.children.flatMap(n=>n.all())];}
  }
  globalThis.document={createElement:tag=>new Node(tag),activeElement:null};
  const root=new Node("section"), calls=[], reads=[];
  let locale = "zh";
  let snapshot={candidates:[candidate("a"),{...candidate("b"),status:"deferred"},{...candidate("c"),status:"rejected"}],syntheses:[],policies:[{clientId:"client",submitCandidates:false}],activity:[],organization:{available:true}};
  const token = value => createHash("sha256").update(JSON.stringify(value)).digest("hex");
  const synthesisToken = full => token([full, snapshot.candidates.filter(item=>full.candidates.some(ref=>ref.candidateId===item.candidateId)), snapshot.synthesisContexts?.[full.contextId] || full.comparedPages || []]);
  const view=createContextHub({root, request:async path=>{
    reads.push(path);
    const replacement = options.request?.(path);
    if (replacement !== undefined) return replacement;
    if (path === "/api/context-hub/inbox") {
      const summary = structuredClone(snapshot);
      delete summary.activity; delete summary.policies; delete summary.synthesisContexts;
      summary.candidates.forEach(item => {item.detailToken=token(item);delete item.experience; delete item.promotionRequest; item.summaryOnly = true;});
      summary.syntheses.forEach(item => {
        item.detailToken=synthesisToken(item);
        delete item.narrative; delete item.comparedPages; delete item.unresolved; item.summaryOnly = true;
        if (item.automaticReview) {delete item.automaticReview.input; delete item.automaticReview.steps;}
        for (const output of [...(item.outputs || []), ...(item.reviewRequest?.outputs || [])]) delete output.content;
      });
      return summary;
    }
    if (path === "/api/context-hub/activity") return {activity:structuredClone(snapshot.activity), limits:{topicsPerClientScope:12,totalCards:192}};
    if (path === "/api/context-hub/policies") return {policies:structuredClone(snapshot.policies)};
    if (path.startsWith("/api/context-hub/candidate/")) {
      const full=snapshot.candidates.find(item => item.candidateId === decodeURIComponent(path.split("/").at(-1)));
      return {candidate:structuredClone(full),detailToken:token(full)};
    }
    if (path.startsWith("/api/context-hub/synthesis/")) {
      const full = snapshot.syntheses.find(item => item.synthesisId === decodeURIComponent(path.split("/").at(-1)));
      const sources=snapshot.candidates.filter(item=>full.candidates.some(ref=>ref.candidateId===item.candidateId));
      return structuredClone({synthesis:full, candidates:sources, synthesisContext:snapshot.synthesisContexts?.[full.contextId] || full.comparedPages || [],detailToken:synthesisToken(full),candidateDetailTokens:Object.fromEntries(sources.map(item=>[item.candidateId,token(item)]))});
    }
    if (path === "/api/enrollment") return {result:{registrations:[]}};
    throw new Error(`Unexpected read: ${path}`);
  },mutate:async(...args)=>{calls.push(args);return options.mutate?.(...args) || {status:"promoted",outputs:[]};},confirmAction:options.confirmAction || (async()=>false),icon:()=>new Node("svg"),language:()=>locale,scopeName:value=>({"project:wiki":"Mini Wiki","project:math":"Math Workspace"})[value] || value,openPage(){},formatTime:value=>value||"-"});
  const find=(tag,text)=>root.all().find(n=>n.tagName===tag&&n.textContent===text);
  const expand=async key=>{const detail=root.querySelectorAll("details[data-disclosure]").find(item=>item.dataset.disclosure===key);assert.ok(detail,`Disclosure ${key} exists`);detail.open=true;await detail.fire("toggle");};
  return {root,view,calls,reads,find,expand,get snapshot(){return snapshot;},set language(value){locale=value;}};
}

test("candidate lanes keep deferred evidence and terminal records out of the current queue",async()=>{
  assert.equal(candidateQueue({...candidate("a"),status:"promoting"}),"current");
  const f=hubFixture();await f.view.load();
  const current=f.root.all().find(n=>n.className==="context-current");
  assert.equal(current.children.length,1);
  assert.equal(f.root.querySelectorAll("details[data-disclosure]").find(n=>n.dataset.disclosure==="waiting").children.length,2);
  assert.equal(f.root.querySelectorAll("details[data-disclosure]").find(n=>n.dataset.disclosure==="completed").children.length,2);
  assert.equal(f.calls.length,0);
});

test("experience view separates observations, provisional explanations and producer receipts",async()=>{
  const f=hubFixture();
  f.snapshot.candidates[0].experience={attempt:"Change wait deadline",observation:"The call returned",interpretation:"May reduce timeouts",unresolved:["Long-term rate?"],receipts:[{stage:"inference",outcome:"succeeded",summary:"Execution completed, not task success",source:{providerId:"infer-runtime",locator:"response:1"}}]};
  await f.view.load();
  assert.doesNotMatch(f.root.textContent,/观察结果: The call returned/);
  await f.expand("candidate:a:1");
  assert.match(f.root.textContent,/观察结果: The call returned/);
  assert.match(f.root.textContent,/当前解释（待验证）: May reduce timeouts/);
  assert.match(f.root.textContent,/未决点: Long-term rate\?/);
  assert.match(f.root.textContent,/执行完成、验证通过和任务成功分别记录/);
  assert.equal(f.calls.length,0);
});

test("organization shows the failure category and retry time while preserving raw details",async()=>{
  const f=hubFixture();
  const error="Infer Runtime maintenance response wait timed out after 130s; execution outcome unknown";
  Object.assign(f.snapshot.organization,{error,failureKind:"response_timeout",nextAttemptAt:"2026-09-21T02:27:50Z"});
  await f.view.load();
  assert.match(f.root.textContent,/等待响应超时，结果未确认/);
  assert.match(f.root.textContent,/下次重试检查: 2026-09-21T02:27:50Z/);
  assert.ok(f.root.textContent.includes(error));
  assert.equal(f.calls.length,0,"viewing diagnostics does not submit a retry");
});

test("permission edits survive refresh and locale rerenders until explicitly saved",async()=>{
  const f=hubFixture();await f.view.load();await f.view.openPolicies();
  const submit=f.root.all().find(n=>n.tagName==="LABEL"&&n.textContent==="提交候选").firstChild;
  submit.checked=true;await submit.fire("change");
  assert.equal(f.view.hasUnsavedChanges(),true);
  await f.view.load();f.view.render();
  const refreshed=f.root.all().find(n=>n.tagName==="LABEL"&&n.textContent==="提交候选").firstChild;
  assert.equal(refreshed.checked,true);assert.equal(f.calls.length,0);
  await f.find("BUTTON","保存权限").fire("click");
  assert.equal(f.calls[0][1].params.submitCandidates,true);
  assert.equal(f.view.hasUnsavedChanges(),false);
});

test("candidate refresh retains edits only while their evidence version is current",async()=>{
  const f=hubFixture();await f.view.load();await f.find("BUTTON","接受并编辑").fire("click");
  let input=f.root.all().find(n=>n.tagName==="LABEL"&&n.textContent==="标题").lastChild;
  input.value="My draft";await input.fire("input");await f.view.load();
  input=f.root.all().find(n=>n.tagName==="LABEL"&&n.textContent==="标题").lastChild;
  assert.equal(input.value,"My draft");assert.equal(f.view.hasUnsavedChanges(),true);
  f.snapshot.candidates[0].version++;await f.view.load();
  assert.equal(f.view.hasUnsavedChanges(),false);assert.match(f.root.textContent,/依据已变化/);assert.equal(f.calls.length,0);
});

test("canceling candidate edits confirms discard while keeping staged decisions and performing no writes", async () => {
  let discard=false;const prompts=[];
  const f=hubFixture({confirmAction:async options=>{prompts.push(options);return discard;}});
  await f.view.load();await f.find("BUTTON","接受并编辑").fire("click");
  let input=f.root.all().find(item=>item.dataset.focusKey==="candidate-editor:title");
  input.value="Preserved draft";await input.fire("input");
  assert.match(f.root.textContent,/有未保存更改/);
  await f.find("BUTTON","取消").fire("click");
  input=f.root.all().find(item=>item.dataset.focusKey==="candidate-editor:title");
  assert.equal(input.value,"Preserved draft");assert.equal(f.view.hasUnsavedChanges(),true);
  assert.match(prompts[0].description,/已暂存/);
  discard=true;await f.find("BUTTON","取消").fire("click");
  assert.equal(f.view.hasUnsavedChanges(),false);assert.equal(f.calls.length,0);
});

test("canceling a synthesis editor uses the same retained-draft confirmation", async () => {
  let discard=false;
  const f=hubFixture({confirmAction:async()=>discard});
  f.snapshot.syntheses=[completeSynthesis()];await f.view.load();await f.expand("synthesis:s1:1");
  await f.find("BUTTON","编辑记忆草案").fire("click");
  let content=f.root.all().find(item=>item.dataset.focusKey==="synthesis-editor:0:content");
  content.value="Human synthesis draft";await content.fire("input");
  await f.find("BUTTON","取消").fire("click");
  assert.equal(f.root.all().find(item=>item.dataset.focusKey==="synthesis-editor:0:content").value,"Human synthesis draft");
  discard=true;await f.find("BUTTON","取消").fire("click");
  assert.equal(f.view.hasUnsavedChanges(),false);assert.equal(f.calls.length,0);
});

test("summary projections cannot be staged as complete review evidence",()=>{
  assert.throws(()=>reviewDraft([{...candidate("a"),summaryOnly:true}],"reject"),/complete candidate evidence/);
  assert.throws(()=>synthesisDraft({...synthesis(),summaryOnly:true},[candidate("a"),candidate("b")],[memory()]),/complete synthesis/);
});

test("opening activity uses its own endpoint without inbox, enrollment or review history",async()=>{
  const f=hubFixture();
  f.snapshot.activity=[{cardId:"act",clientId:"client",scope:"project:math",topicKey:"math:plugin",summary:"提交和插件验证已完成，真实侧栏仍需重启。",version:1,updatedAt:"2026-10-22T12:00:00Z",expiresAt:"2026-10-24T12:00:00Z",title:"数学工作区侧栏",status:"waiting_user",nextStep:"请重启 Codex 后核对真实侧栏。"}];
  await f.view.openActivity();
  assert.deepEqual(f.reads,["/api/context-hub/activity"]);
  assert.match(f.root.textContent,/数学工作区侧栏/);assert.match(f.root.textContent,/待用户/);assert.match(f.root.textContent,/下一步请重启 Codex/);
  assert.ok(f.root.textContent.includes(f.snapshot.activity[0].summary));
  assert.match(f.root.textContent,/过期不代表任务结束/);assert.match(f.root.textContent,/Math Workspace/);
  assert.equal(f.calls.length,0);
  f.language="en";f.view.render();
  assert.match(f.root.textContent,/Waiting for you/);assert.match(f.root.textContent,/Next step/);
  assert.ok(f.root.textContent.includes(f.snapshot.activity[0].summary));
  assert.equal(f.reads.length,1,"locale changes keep the same activity snapshot");
});

test("activity project filter retains six topics, ordering, focus and explicit child attention",async()=>{
  const f=hubFixture();
  const base={clientId:"client",scope:"project:wiki",summary:"保留原始说明。",version:1,expiresAt:"2026-10-24T12:00:00Z"};
  f.snapshot.activity=Array.from({length:6},(_,i)=>({...base,cardId:`a${i}`,topicKey:`wiki:${i}`,title:`主题 ${i}`,updatedAt:`2026-10-22T${10+i}:00:00Z`}));
  f.snapshot.activity[5].parentTopicKey="wiki:0";f.snapshot.activity[5].status="waiting_dependency";f.snapshot.activity[5].nextStep="主任务集成云模块并验收画面";
  f.snapshot.activity.push({...base,scope:"project:math",cardId:"m",topicKey:"math",title:"其他项目",updatedAt:"2026-10-22T16:00:00Z"});
  await f.view.openActivity();
  assert.match(f.root.textContent,/7 个主题/);
  let select=f.root.all().find(item=>item.tagName==="SELECT");select.value="project:wiki";select.focus();await select.fire("change");
  assert.match(f.root.textContent,/6 个主题/);assert.doesNotMatch(f.root.textContent,/其他项目/);
  const cards=f.root.all().filter(item=>item.tagName==="ARTICLE");
  assert.match(cards[0].textContent,/主题 0/);assert.match(cards[0].textContent,/下一步主任务集成云模块/);
  const children=f.root.querySelectorAll("details[data-disclosure]").find(item=>item.dataset.disclosure==="activity-children:a0");
  assert.equal(children.open,true,"explicit waiting child is visible on first render");
  children.open=false;await f.view.load();
  select=f.root.all().find(item=>item.tagName==="SELECT");
  assert.equal(select.value,"project:wiki");assert.equal(document.activeElement,select);
  assert.equal(f.root.querySelectorAll("details[data-disclosure]").find(item=>item.dataset.disclosure==="activity-children:a0").open,false,"operator collapse survives refresh");
  assert.deepEqual(f.reads,["/api/context-hub/activity","/api/context-hub/activity"]);
});

const completeSynthesis = (fields = {}) => ({...synthesis(),title:"记忆综合草稿",scope:"a",maturity:"ready",narrative:"完整叙事证据",reason:"保留原始依据",unresolved:[],outputs:[memory()],results:[],...fields});

test("synthesis history and review evidence load on expansion while withdrawal actions remain",async()=>{
  const f=hubFixture();
  f.snapshot.candidates=[{...candidate("a"),version:2,status:"promoted"},{...candidate("b"),version:2,status:"promoted"}];
  f.snapshot.syntheses=[completeSynthesis({status:"promoted",reviewRequest:{outputs:[memory()]},results:[{pageId:"page-a"}],
    automaticReview:{state:"written",reason:"独立复核通过",authorized:true,decisions:[{outputIndex:0,verdict:"approve",reason:"完整覆盖"}],input:{pages:[{pageId:"existing",revisionId:"rev1",content:"完整比较页面原文"}]},steps:[{tier:"upgraded",stage:"review",state:"completed"}]}})];
  await f.view.load();
  assert.deepEqual(f.reads,["/api/context-hub/inbox","/api/enrollment"]);
  assert.doesNotMatch(f.root.textContent,/完整比较页面原文|完整叙事证据|撤回这次自动写入/);
  await f.expand("completed");assert.equal(f.reads.length,2);
  await f.expand("synthesis:s1:1");
  assert.deepEqual(f.reads,["/api/context-hub/inbox","/api/enrollment","/api/context-hub/synthesis/s1"]);
  assert.match(f.root.textContent,/完整比较页面原文/);assert.match(f.root.textContent,/完整叙事证据/);
  assert.ok(f.find("BUTTON","撤回这次自动写入"));
  assert.equal(f.calls.length,0,"inspecting history never withdraws or resubmits writes");
});

test("loaded synthesis source changes invalidate staged outputs without affecting fresh candidate selection",async()=>{
  const f=hubFixture();f.snapshot.candidates=[candidate("a"),candidate("b"),candidate("c")];f.snapshot.syntheses=[completeSynthesis()];
  await f.view.load();await f.expand("synthesis:s1:1");
  await f.find("BUTTON","编辑记忆草案").fire("click");
  await f.find("BUTTON","暂存整理决定").fire("click");
  assert.equal(f.view.hasUnsavedChanges(),true);
  const checkbox=f.root.all().find(item=>item.tagName==="INPUT"&&item.dataset.focusKey==="c");checkbox.checked=true;checkbox.focus();await checkbox.fire("change");
  f.snapshot.syntheses[0].version++;await f.view.load();
  assert.equal(f.view.hasUnsavedChanges(),false);assert.match(f.root.textContent,/依据已变化/);
  const refreshed=f.root.all().find(item=>item.tagName==="INPUT"&&item.dataset.focusKey==="c");
  assert.equal(refreshed.checked,true);assert.equal(document.activeElement,refreshed);
  assert.equal(f.calls.length,0);
});

test("stale detail responses cannot reopen an editor after a newer inbox snapshot",async()=>{
  let resolveDetail;
  const delayed=new Promise(resolve=>{resolveDetail=resolve;});
  const f=hubFixture({request:path=>path==="/api/context-hub/candidate/a"?delayed:undefined});
  await f.view.load();
  const editing=f.find("BUTTON","接受并编辑").fire("click");
  await new Promise(resolve=>setImmediate(resolve));
  f.snapshot.candidates[0].version=2;await f.view.load();
  resolveDetail({candidate:candidate("a")});await editing;
  assert.equal(f.view.hasUnsavedChanges(),false);
  assert.equal(f.root.all().some(item=>item.className==="context-card context-editor"),false);
  assert.equal(f.calls.length,0);
});

test("a pending or failed inbox read does not block activity or replace its errors",async()=>{
  let rejectInbox;
  const delayed=new Promise((_,reject)=>{rejectInbox=reject;});
  const f=hubFixture({request:path=>path==="/api/context-hub/inbox"?delayed:undefined});
  const inbox=f.view.load();await new Promise(resolve=>setImmediate(resolve));
  await f.view.openActivity();
  assert.equal(f.root.querySelector("#context-tab-activity").attributes["aria-selected"],"true");
  assert.equal(document.activeElement,f.root.querySelector("#context-tab-activity"));
  rejectInbox(new Error("Old inbox request failed"));await inbox;
  assert.doesNotMatch(f.root.textContent,/Old inbox request failed/);
  assert.match(f.root.textContent,/当前没有共享近况/);
  assert.equal(f.calls.length,0);
});

test("candidate retry uses only the complete original promotion decision",async()=>{
  const f=hubFixture();
  const original={candidates:[{candidateId:"a",version:1}],action:"promote",title:"Reviewed title",content:"Exact committed request"};
  f.snapshot.candidates=[{...candidate("a"),status:"promoting",promotionRequest:original}];
  await f.view.load();assert.deepEqual(f.reads,["/api/context-hub/inbox","/api/enrollment"]);
  await f.find("BUTTON","重试").fire("click");
  assert.equal(f.reads[2],"/api/context-hub/candidate/a");
  assert.deepEqual(f.calls[0],["/api/context-hub",{operation:"review",params:original}]);
});

test("detail read failures retain the summary and offer a bounded read retry without mutation",async()=>{
  let failed=true;
  const f=hubFixture({request:path=>path==="/api/context-hub/candidate/a"&&failed?Promise.reject(new Error("Detail unavailable")):undefined});
  await f.view.load();await f.expand("candidate:a:1");
  assert.match(f.root.textContent,/Detail unavailable/);assert.match(f.root.textContent,/Evidence/);assert.equal(f.calls.length,0);
  const before=f.reads.length;f.view.render();assert.equal(f.reads.length,before,"rerendering does not automatically retry a failed evidence read");
  failed=false;await f.find("BUTTON","重试读取完整依据").fire("click");
  assert.doesNotMatch(f.root.textContent,/Detail unavailable/);assert.equal(f.calls.length,0);
});

test("candidate editors retain text, selection and focus while changing locale",async()=>{
  const f=hubFixture();await f.view.load();await f.find("BUTTON","接受并编辑").fire("click");
  const field=f.root.all().find(item=>item.dataset.focusKey==="candidate-editor:title");
  field.value="Unsubmitted title";await field.fire("input");field.selectionStart=3;field.selectionEnd=7;field.focus();
  f.language="en";f.view.render();
  assert.equal(document.activeElement.dataset.focusKey,"candidate-editor:title");
  assert.equal(document.activeElement.value,"Unsubmitted title");assert.equal(f.view.hasUnsavedChanges(),true);
  assert.equal(f.calls.length,0);
});

test("lightweight inbox keeps enrolled client display names while technical identity stays inspectable",async()=>{
  const f=hubFixture({request:path=>path==="/api/enrollment"?{result:{registrations:[{client:{principal:{principalId:"registered-client",displayName:"Registered client"}}}]}}:undefined});
  f.snapshot.candidates[0].clientId="registered-client";
  await f.view.load();
  const meta=f.root.all().find(item=>item.className==="context-meta"&&item.textContent.includes("Registered client"));
  assert.ok(meta);assert.match(f.root.textContent,/registered-client/);
  assert.deepEqual(f.reads,["/api/context-hub/inbox","/api/enrollment"]);
  await f.view.openActivity();assert.equal(f.reads.filter(path=>path==="/api/enrollment").length,1);
});

test("unchanged detail tokens reuse complete evidence across inbox refreshes",async()=>{
  const f=hubFixture();f.snapshot.syntheses=[completeSynthesis()];
  await f.view.load();await f.expand("synthesis:s1:1");
  assert.equal(f.reads.filter(path=>path==="/api/context-hub/synthesis/s1").length,1);
  await f.view.load();await f.expand("synthesis:s1:1");
  assert.equal(f.reads.filter(path=>path==="/api/context-hub/synthesis/s1").length,1,"stable metadata does not download the same evidence again");
  assert.match(f.root.textContent,/完整叙事证据/);assert.equal(f.calls.length,0);
});

test("same-version automatic review transitions discard stale audit bodies and retain fresh summaries",async()=>{
  const f=hubFixture();f.snapshot.syntheses=[completeSynthesis({automaticReview:{state:"running",reason:"Review running",steps:[{tier:"sol",stage:"candidate_review",state:"running",requestId:"old-audit"}]}})];
  await f.view.load();await f.expand("synthesis:s1:1");assert.match(f.root.textContent,/old-audit/);
  Object.assign(f.snapshot.syntheses[0].automaticReview,{state:"waiting_budget",reason:"Budget waiting now",steps:[{tier:"sol",stage:"candidate_review",state:"waiting",requestId:"new-audit"}]});
  await f.view.load();assert.doesNotMatch(f.root.textContent,/old-audit/);assert.match(f.root.textContent,/Budget waiting now/);
  await f.expand("synthesis:s1:1");assert.match(f.root.textContent,/new-audit/);assert.doesNotMatch(f.root.textContent,/old-audit/);
  assert.equal(f.reads.filter(path=>path==="/api/context-hub/synthesis/s1").length,2);
  assert.equal(f.snapshot.syntheses[0].version,1);assert.equal(f.calls.length,0);
});

test("same-version in-flight receipts refresh the exact resume progress",async()=>{
  const f=hubFixture();f.snapshot.candidates=[{...candidate("a"),status:"promoting"},{...candidate("b"),status:"promoting"}];
  f.snapshot.syntheses=[completeSynthesis({status:"promoting",reviewRequest:{synthesisId:"s1",version:1,outputs:[memory()]},results:[]})];
  await f.view.load();await f.expand("synthesis:s1:1");assert.match(f.root.textContent,/已完成 0 项输出/);
  f.snapshot.syntheses[0].results.push({pageId:"page-a",revisionId:"rev-a"});
  await f.view.load();await f.expand("synthesis:s1:1");
  assert.match(f.root.textContent,/已完成 1 项输出/);assert.doesNotMatch(f.root.textContent,/已完成 0 项输出/);
  assert.equal(f.snapshot.syntheses[0].version,1);assert.equal(f.calls.length,0);
});

test("cross-window withdrawal invalidates a promoted detail even when every older summary field stays unchanged",async()=>{
  const f=hubFixture();f.snapshot.candidates=[{...candidate("a"),version:2,status:"promoted"},{...candidate("b"),version:2,status:"promoted"}];
  f.snapshot.syntheses=[completeSynthesis({status:"promoted",reviewRequest:{outputs:[memory()]},results:[{pageId:"page-a"}],automaticReview:{state:"written",reason:"Reviewed and written",authorized:true,decisions:[{outputIndex:0,verdict:"approve"}],undoResults:[]}})];
  await f.view.load();await f.expand("synthesis:s1:1");assert.ok(f.find("BUTTON","撤回这次自动写入"));
  f.snapshot.syntheses[0].automaticReview.undoResults.push({outputIndex:0,status:"archived",pageId:"page-a"});
  await f.view.load();await f.expand("synthesis:s1:1");
  assert.match(f.root.textContent,/已撤回，历史内容保留/);assert.equal(f.find("BUTTON","撤回这次自动写入"),undefined);
  assert.equal(f.reads.filter(path=>path==="/api/context-hub/synthesis/s1").length,2);
  assert.equal(f.calls.length,0);
});

test("a detail from a newer same-version state is rejected until its summary is refreshed",async()=>{
  const f=hubFixture();f.snapshot.syntheses=[completeSynthesis({automaticReview:{state:"running",reason:"Original summary",steps:[]}})];
  await f.view.load();f.snapshot.syntheses[0].automaticReview.reason="New full evidence reason";
  await f.expand("synthesis:s1:1");
  assert.match(f.root.textContent,/依据已变化，请刷新后重新审阅/);
  assert.doesNotMatch(f.root.textContent,/New full evidence reason|完整叙事证据/);
  assert.equal(f.find("BUTTON","编辑记忆草案"),undefined);assert.equal(f.calls.length,0);
  await f.view.load();await f.expand("synthesis:s1:1");
  assert.match(f.root.textContent,/New full evidence reason|完整叙事证据/);
  assert.ok(f.find("BUTTON","编辑记忆草案"));
});

test("candidate same-version pending-to-promoting replaces cached status and retrieves its exact saved request",async()=>{
  const f=hubFixture();f.snapshot.candidates=[candidate("a")];
  await f.view.load();await f.expand("candidate:a:1");
  const original={candidates:[{candidateId:"a",version:1}],action:"promote",title:"Original title",content:"Exact original write plan"};
  f.snapshot.candidates[0].status="promoting";f.snapshot.candidates[0].promotionRequest=original;
  await f.view.load();assert.equal(f.find("BUTTON","接受并编辑"),undefined);assert.ok(f.find("BUTTON","重试"));
  await f.find("BUTTON","重试").fire("click");
  assert.deepEqual(f.calls[0],["/api/context-hub",{operation:"review",params:original}]);
  assert.equal(f.reads.filter(path=>path==="/api/context-hub/candidate/a").length,2);
});

test("same-version review changes retain synthesis edits and require another inspection before staging",async()=>{
  const f=hubFixture();f.snapshot.syntheses=[completeSynthesis({automaticReview:{state:"running",reason:"Original evidence",steps:[]}})];
  await f.view.load();await f.expand("synthesis:s1:1");await f.find("BUTTON","编辑记忆草案").fire("click");
  let content=f.root.all().find(item=>item.dataset.focusKey==="synthesis-editor:0:content");content.value="Human edited output";await content.fire("input");
  f.snapshot.syntheses[0].automaticReview.state="waiting_budget";f.snapshot.syntheses[0].automaticReview.reason="New review receipt";
  await f.view.load();await f.find("BUTTON","暂存整理决定").fire("click");
  content=f.root.all().find(item=>item.dataset.focusKey==="synthesis-editor:0:content");assert.equal(content.value,"Human edited output");
  assert.match(f.root.textContent,/编辑内容已保留/);assert.doesNotMatch(f.root.textContent,/整理草案 · Decision.*待提交/);
  assert.equal(f.calls.length,0);
  await f.find("BUTTON","暂存整理决定").fire("click");
  assert.match(f.root.textContent,/待提交/);assert.doesNotMatch(f.root.textContent,/编辑内容已保留/);
  assert.equal(f.calls.length,0);
});
