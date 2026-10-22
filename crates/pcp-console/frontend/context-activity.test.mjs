import test from "node:test";
import assert from "node:assert/strict";
import { activityGroups, activityStatusLabel, activityTitle } from "../src/context-activity.js";

const card = (topicKey, hour, fields = {}) => ({cardId:topicKey, clientId:"chatgpt", scope:"project:wiki", topicKey,
  summary:`${topicKey}的完整原文。下一句保留。`, updatedAt:`2026-10-22T${String(hour).padStart(2,"0")}:00:00Z`, ...fields});

test("legacy titles use a readable first sentence without changing summary or implying status", () => {
  const old = card("mini-wiki:cloud", 10, {summary:"云模块完成。仍待主任务集成与画面验收。"});
  assert.equal(activityTitle(old), "云模块完成。");
  assert.equal(old.summary, "云模块完成。仍待主任务集成与画面验收。");
  assert.equal(activityStatusLabel(old.status), "");
  assert.equal(activityTitle({...old,title:"大气场景：云"}), "大气场景：云");
  assert.equal(activityTitle({...old,summary:"",topicKey:"mini-wiki:atmosphere-cloud"}), "atmosphere cloud");
});

test("all topics remain visible and both projects and sibling topics sort by latest update", () => {
  const cards = [card("old", 1), ...Array.from({length:6}, (_, i) => card(`wiki:${i}`, i + 2)), card("math", 20, {scope:"project:math"})];
  const before = structuredClone(cards), groups = activityGroups(cards);
  assert.equal(groups[0].scope,"project:math");
  assert.equal(groups[1].count,7);
  assert.deepEqual(groups[1].roots.map(record=>record.item.topicKey),["wiki:5","wiki:4","wiki:3","wiki:2","wiki:1","wiki:0","old"]);
  assert.equal(activityGroups(cards,"project:wiki").length,1);
  assert.deepEqual(cards,before);
});

test("explicit parent references group within the same client and Scope and order by descendant updates", () => {
  const cards = [card("main", 1), card("other", 10), card("cloud", 20, {parentTopicKey:"main"}), card("ground", 4, {parentTopicKey:"main"}),
    card("foreign-client", 21, {clientId:"other",parentTopicKey:"main"}), card("foreign-scope", 22, {scope:"project:math",parentTopicKey:"main"})];
  const wiki = activityGroups(cards).find(group=>group.scope==="project:wiki");
  assert.deepEqual(wiki.roots.map(record=>record.item.topicKey),["foreign-client","main","other"]);
  assert.deepEqual(wiki.roots[1].children.map(record=>record.item.topicKey),["cloud","ground"]);
  assert.equal(wiki.roots[1].updatedAt,Date.parse(cards[2].updatedAt));
  assert.equal(activityGroups(cards)[0].roots[0].children.length,0);
});

test("missing parents and parent cycles preserve every card without recursive loops", () => {
  const cards = [card("a", 1,{parentTopicKey:"b"}),card("b", 2,{parentTopicKey:"a"}),card("c", 3,{parentTopicKey:"c"}),card("d", 4,{parentTopicKey:"missing"}),card("e", 5,{parentTopicKey:"a"})];
  const group = activityGroups(cards)[0];
  assert.equal(group.roots.length,5);
  assert.equal(group.count,5);
  assert.ok(group.roots.every(record=>record.children.length===0));
});

test("waiting labels require exact publisher states in either locale", () => {
  assert.equal(activityStatusLabel("waiting_user"),"待用户");
  assert.equal(activityStatusLabel("waiting_dependency",true),"Awaiting integration / dependency");
  assert.equal(activityStatusLabel("completed",true),"Completed");
  assert.equal(activityStatusLabel(undefined,true),"");
});
