import test from "node:test";
import assert from "node:assert/strict";
import { PageEditSession, createPageEditor } from "../src/page-editor.js";

test("editor reads a fresh full payload and saves only text against that revision", async () => {
  const calls = [];
  const session = new PageEditSession({
    request: async (url) => { calls.push(url); return {pageId:"pg_1", revisionId:"rev_1", content:"old"}; },
    mutate: async (url, body) => { calls.push({url,body}); return {pageId:"pg_1",revisionId:"rev_2"}; },
  });
  await session.open("pg_1");
  assert.equal(session.dirty, false);
  assert.equal(await session.save(), null);
  session.content = "fixed";
  await session.save();
  assert.deepEqual(calls, ["/api/pages/pg_1/edit", {url:"/api/pages/pg_1/edit",body:{expectedRevisionId:"rev_1",content:"fixed"}}]);
  assert.equal(session.dirty, false);
  assert.equal(session.snapshot.revisionId, "rev_2");
});

test("revision conflict keeps the entire draft and clears running state", async () => {
  const session = new PageEditSession({
    request: async () => ({pageId:"pg_1",revisionId:"rev_1",content:"original"}),
    mutate: async () => { throw new Error("revision conflict"); },
  });
  await session.open("pg_1"); session.content = "my draft";
  await assert.rejects(session.save(), /revision conflict/);
  assert.equal(session.content,"my draft"); assert.equal(session.dirty,true); assert.equal(session.busy,false);
  assert.equal(session.snapshot.revisionId,"rev_1");
});

test("double submission cannot publish the same draft twice", async () => {
  let resolve; let writes = 0;
  const session = new PageEditSession({
    request: async () => ({pageId:"pg_1",revisionId:"rev_1",content:"original"}),
    mutate: () => { writes++; return new Promise(r => {resolve = r;}); },
  });
  await session.open("pg_1"); session.content = "fixed";
  const pending = session.save();
  assert.equal(session.busy,true); assert.equal(await session.save(),null);
  resolve({revisionId:"rev_2"}); await pending;
  assert.equal(writes,1); assert.equal(session.busy,false);
});

test("the Page editor reports dirty and in-flight state and canceled leave retains exact text", async () => {
  class Node {
    constructor() { this.children=[];this.handlers={};this.dataset={};this.classList={toggle(){}};this.value="";this.hidden=false; }
    append(...nodes) {this.children.push(...nodes);}
    setAttribute() {}
    addEventListener(name,handler) {this.handlers[name]=handler;}
    focus() {}
  }
  const nodes=new Map(), byId=id=>{if(!nodes.has(id))nodes.set(id,new Node());return nodes.get(id);};
  globalThis.document={getElementById:byId,createElementNS:()=>new Node()};
  let discard=false,finishRead,writes=0;
  const editor=createPageEditor({request:()=>new Promise(resolve=>{finishRead=resolve;}),
    mutate:async()=>{writes++;throw new Error("revision conflict");},confirmAction:async()=>discard,t:value=>value,
    onSaved:async()=>{},onDeleted:async()=>{}});
  editor.attach({page:{pageId:"one"},actions:{canEdit:true,canDelete:true}});
  const loading=editor.begin();assert.equal(editor.hasUnsavedChanges(),true);
  assert.equal(await editor.leave(),false,"an in-flight editor read is protected");
  finishRead({pageId:"one",revisionId:"rev_one",content:"original"});await loading;
  assert.equal(editor.hasUnsavedChanges(),false);
  const content=byId("page-editor-content");content.value="Exact human draft\n保留内容";content.handlers.input();
  assert.equal(editor.hasUnsavedChanges(),true);assert.equal(byId("page-editor-draft-status").hidden,false);
  assert.equal(await editor.leave(),false);assert.equal(content.value,"Exact human draft\n保留内容");
  await byId("page-save").handlers.click();
  assert.equal(writes,1);assert.equal(editor.hasUnsavedChanges(),true);
  assert.equal(content.value,"Exact human draft\n保留内容");
  discard=true;assert.equal(await editor.leave(),true);assert.equal(editor.hasUnsavedChanges(),false);
});
