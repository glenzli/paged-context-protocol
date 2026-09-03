import test from "node:test";
import assert from "node:assert/strict";
import { pendingCandidate, reviewDraft, reconcileDrafts } from "../src/context-hub.js";
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
