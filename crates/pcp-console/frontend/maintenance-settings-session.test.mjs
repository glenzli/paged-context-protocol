import test from "node:test";
import assert from "node:assert/strict";
import { MaintenanceSettingsSession } from "../src/maintenance-settings-session.js";

function fixture({ request = async () => ({enabled:false, threshold:8}), mutate = async () => {} } = {}) {
  let draft = null, renders = 0;
  const session = new MaintenanceSettingsSession({ request, mutate,
    readDraft: () => draft, populate: value => { draft = structuredClone(value); },
    snapshotDraft: value => value, changed: () => { renders++; } });
  return { session, get draft() { return draft; }, get renders() { return renders; } };
}

test("opening settings is local; a failed lazy load can be retried", async () => {
  let reads = 0, fail = true;
  const f = fixture({ request: async () => { reads++; if (fail) throw new Error("offline"); return {enabled:false, threshold:8}; } });
  f.session.open();
  assert.equal(reads, 0);
  await f.session.load();
  assert.equal(f.session.loaded, false); assert.match(f.session.error.message, /offline/);
  fail = false; await f.session.load();
  assert.equal(f.session.loaded, true); assert.equal(f.session.dirty, false); assert.equal(f.session.loading, false);
});

test("late settings loads cannot overwrite a draft or a reopened session", async () => {
  const requests = [];
  const f = fixture({ request: () => new Promise(resolve => requests.push(resolve)) });
  f.session.open(); let loading = f.session.load();
  requests[0]({enabled:false, threshold:8}); await loading;
  loading = f.session.load();
  f.draft.threshold = 17; f.session.touch();
  requests[1]({enabled:true, threshold:9}); await loading;
  assert.equal(f.draft.threshold, 17); assert.equal(f.draft.enabled, false); assert.equal(f.session.dirty, true);
  const old = f.session.load(); f.session.close(); f.session.open();
  const current = f.session.load();
  requests[3]({enabled:true, threshold:20}); await current;
  requests[2]({enabled:false, threshold:3}); await old;
  assert.equal(f.draft.threshold, 20); assert.equal(f.session.dirty, false);
});

test("uncertain saves retain and freeze the exact draft until explicit read-only reconciliation", async () => {
  const calls = [];
  let current = {enabled:false, threshold:8};
  const f = fixture({ request: async path => { calls.push(["read", path]); return structuredClone(current); },
    mutate: async (path, body) => { calls.push(["write", path, body]); throw new Error("lost response"); } });
  f.session.open(); await f.session.load(); f.draft.threshold = 19;
  assert.equal(await f.session.save(), false);
  assert.equal(f.session.uncertain, true); assert.equal(f.session.protected, true); assert.equal(f.draft.threshold, 19);
  assert.equal(await f.session.save(), false); assert.equal(calls.filter(call => call[0] === "write").length, 1);
  f.session.suspend(); f.session.open();
  assert.equal(f.session.uncertain, true); assert.equal(f.draft.threshold, 19);
  assert.equal(calls.length, 2, "suspending or reopening does not replay or read automatically");
  await f.session.load({reconcile:true});
  assert.equal(f.session.uncertain, false); assert.equal(f.session.dirty, true); assert.equal(f.draft.threshold, 19);
  assert.match(f.session.notice, /draft is kept/);
  current = {enabled:false, threshold:19}; await f.session.load({reconcile:true});
  assert.equal(f.session.dirty, false); assert.match(f.session.notice, /match/);
  assert.equal(calls.filter(call => call[0] === "write").length, 1);
});

test("failed reconciliation remains recoverable and a known rejection remains editable", async () => {
  let offline = false;
  const f = fixture({ request: async () => { if (offline) throw new Error("offline"); return {enabled:false, threshold:8}; },
    mutate: async () => { throw new Error("lost response"); } });
  f.session.open(); await f.session.load(); f.draft.threshold = 10; await f.session.save();
  offline = true; await f.session.load({reconcile:true});
  assert.equal(f.session.uncertain, true); assert.equal(f.session.loading, false); assert.equal(f.draft.threshold, 10);
  f.session.suspend(); assert.equal(f.session.protected, true);
  f.session.open(); offline = false; await f.session.load({reconcile:true});
  assert.equal(f.session.uncertain, false);
  f.session.mutate = async () => { throw Object.assign(new Error("validation failed"), {mutationOutcome:"not_applied"}); };
  await f.session.save(); assert.equal(f.session.uncertain, false); assert.equal(f.session.dirty, true);
});

test("in-flight settings saves cannot be submitted twice", async () => {
  let finish, writes = 0;
  const f = fixture({ mutate: () => { writes++; return new Promise(resolve => { finish = resolve; }); } });
  f.session.open(); await f.session.load(); f.draft.threshold = 12;
  const saving = f.session.save();
  assert.equal(f.session.protected, true); assert.equal(await f.session.save(), false);
  finish(); assert.equal(await saving, true);
  assert.equal(writes, 1); assert.equal(f.session.dirty, false); assert.equal(f.session.saving, false);
});

test("closing during an uncertain read check keeps recovery available and ignores its late response", async () => {
  const f = fixture({mutate:async()=>{throw new Error("lost save response");}});
  f.session.open();await f.session.load();f.draft.threshold=27;await f.session.save();
  let finish;
  f.session.request=()=>new Promise(resolve=>{finish=resolve;});
  const checking=f.session.load({reconcile:true});
  assert.match(f.session.error.message,/lost save/);
  f.session.suspend();f.session.open();
  finish({enabled:true,threshold:27});await checking;
  assert.equal(f.session.uncertain,true);assert.match(f.session.error.message,/lost save/);
  assert.equal(f.session.loading,false);assert.equal(f.draft.threshold,27);
});
