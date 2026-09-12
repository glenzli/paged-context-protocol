import test from "node:test";
import assert from "node:assert/strict";
import { pendingCandidate, reviewDraft, reconcileDrafts, candidateReviewQueue } from "../src/context-hub.js";
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
