import test from "node:test";
import assert from "node:assert/strict";
import { PageEditSession } from "../src/page-editor.js";

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
