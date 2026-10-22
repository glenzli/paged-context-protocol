import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../src/app.js", import.meta.url), "utf8");
const body = name => {
  const at = source.indexOf(`function ${name}(`), start = source.slice(at - 6, at) === "async " ? at - 6 : at;
  const rest = source.slice(start), end = rest.slice(1).search(/\n(?:async )?function /) + 1;
  return rest.slice(0, end);
};

function fixture({dirty = false, uncertain = false, saving = false, confirm = false} = {}) {
  const calls = [], nodes = new Map();
  const session = {dirty, uncertain, saving, open(){calls.push("open session");}};
  const byId = id => {
    if (!nodes.has(id)) nodes.set(id, {showModal(){calls.push(`show ${id}`);},close(){calls.push(`close ${id}`);},focus(){calls.push(`focus ${id}`);}});
    return nodes.get(id);
  };
  const context = {maintenanceSettingsSession:session, preferencesConfirming:false, activePreferencesTab:"maintenance", byId,
    renderPreferencesTabs(){calls.push("tabs");}, renderMaintenanceSettingsState(){calls.push("render");},
    confirmAction:async options => {calls.push(options); return confirm;}, t:value=>value};
  runInNewContext(`${body("openPreferences")}\n${body("closePreferences")}\nthis.open = openPreferences; this.close = closePreferences;`, context);
  return {context,calls};
}

test("General opens synchronously without a remote dependency", () => {
  const f = fixture(); f.context.open();
  assert.equal(f.context.activePreferencesTab, "general");
  assert.deepEqual(f.calls, ["open session", "tabs", "render", "show preferences-dialog"]);
});

test("settings close keeps a canceled draft and blocks an in-flight save", async () => {
  const f = fixture({dirty:true});
  assert.equal(await f.context.close(), false);
  assert.equal(f.calls.filter(call=>typeof call === "string" && call.startsWith("close")).length, 0);
  assert.equal(f.calls.find(call=>typeof call === "object").title, "Discard unsaved changes?");
  const busy = fixture({dirty:true,saving:true,confirm:true});
  assert.equal(await busy.context.close(), false); assert.equal(busy.calls.length, 0);
});

test("unknown saves can be closed explicitly for later checking without discarding", async () => {
  const f = fixture({dirty:true,uncertain:true,confirm:true});
  assert.equal(await f.context.close(), true);
  assert.equal(f.calls.find(call=>typeof call === "object").confirmLabel, "Close and check later");
  assert.ok(f.calls.includes("close preferences-dialog"));
  assert.match(source, /if \(maintenanceSettingsSession\.uncertain\) maintenanceSettingsSession\.suspend\(\)/);
});

test("refresh and browser close protect each actual draft owner", () => {
  const listener = source.match(/window\.addEventListener\("beforeunload", event => \{([\s\S]*?)\n\}\);/)[1];
  for (const owner of ["context", "page", "settings", "client", "review", "decisions", "consolidation"]) {
    let prevented = false;
    const event = {preventDefault(){prevented = true;}};
    runInNewContext(listener, {event,
      contextHub:{hasUnsavedChanges:()=>owner === "context"}, pageInspector:{hasUnsavedChanges:()=>owner === "page"},
      maintenanceSettingsSession:{protected:owner === "settings"}, clientAccessView:{hasPendingAction:()=>owner === "client"},
      state:{maintenance:{reviewCommitBusy:owner === "review", reviewDecisions:{size:owner === "decisions" ? 1 : 0}, consolidationDrafts:{size:owner === "consolidation" ? 1 : 0}}}});
    assert.equal(prevented, true, owner); assert.equal(event.returnValue, "");
  }
  assert.match(source, /preferences-dialog"\)\.addEventListener\("cancel", event => \{ event\.preventDefault\(\); void closePreferences\(\)/);
});

test("a save failure is focused inside its maintenance panel even if General was selected during the request", async () => {
  const calls=[];
  const context={maintenanceSettingsSession:{saving:false,uncertain:false,loading:false,error:new Error("lost response"),save:async()=>false},
    activePreferencesTab:"general",byId:id=>({value:id.endsWith("quiet")?"10":id.endsWith("max-wait")?"60":"8",focus(){calls.push(id);}}),
    renderPreferencesTabs(){calls.push("tabs");},renderMaintenanceSettingsState(){},t:value=>value};
  runInNewContext(`${body("saveMaintenanceSettings")}\nthis.save = saveMaintenanceSettings;`,context);
  await context.save({preventDefault(){}});
  assert.equal(context.activePreferencesTab,"maintenance");assert.deepEqual(calls,["tabs","maintenance-settings-error"]);
});
