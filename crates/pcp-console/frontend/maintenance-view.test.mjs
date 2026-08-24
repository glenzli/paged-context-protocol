import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const APP_JS = readFileSync(new URL("../src/app.js", import.meta.url), "utf8");
const INDEX_HTML = readFileSync(new URL("../src/index.html", import.meta.url), "utf8");

test("maintenance work is separated into two accessible sub-tabs", () => {
  assert.match(INDEX_HTML, /class="maintenance-workspace-tabs" role="tablist"/);
  for (const [tab, panel] of [
    ["maintenance-convergence-tab", "maintenance-convergence-panel"],
    ["maintenance-governance-tab", "maintenance-governance-panel"],
  ]) {
    const tabMatch = INDEX_HTML.match(new RegExp(`<button\\s+id="${tab}"([^>]*)>`));
    assert.ok(tabMatch, `missing maintenance tab: ${tab}`);
    assert.match(tabMatch[1], /role="tab"/);
    assert.match(tabMatch[1], new RegExp(`aria-controls="${panel}"`));
    assert.match(INDEX_HTML, new RegExp(`<div\\s+id="${panel}"[^>]*role="tabpanel"`));
  }
});

test("archive governance is a tab panel instead of a competing disclosure", () => {
  assert.doesNotMatch(INDEX_HTML, /<details\s+id="maintenance-archive"/);
  assert.match(INDEX_HTML, /<section\s+id="maintenance-archive"/);
  assert.match(INDEX_HTML, /id="maintenance-governance-panel"[\s\S]*id="archive-start"/);
});

test("sub-tab selection persists without resetting either workflow", () => {
  const setterStart = APP_JS.indexOf("function setMaintenanceWorkspaceTab(");
  const setterEnd = APP_JS.indexOf("\n}\n\nfunction copyStatusPill", setterStart) + 2;
  const setter = APP_JS.slice(setterStart, setterEnd);
  assert.match(APP_JS, /MAINTENANCE_WORKSPACE_STORAGE_KEY/);
  assert.ok(setterStart >= 0 && setterEnd > setterStart);
  assert.match(setter, /writePreference\(MAINTENANCE_WORKSPACE_STORAGE_KEY, workspaceTab\)/);
  assert.doesNotMatch(setter, /resetMaintenanceSession\(/);
  assert.doesNotMatch(setter, /resetArchive/);
});

test("sub-tabs support pointer and standard horizontal keyboard navigation", () => {
  assert.match(APP_JS, /tab\.addEventListener\("click"/);
  for (const key of ["ArrowLeft", "ArrowRight", "Home", "End"]) {
    assert.match(APP_JS, new RegExp(`"${key}"`));
  }
  assert.match(APP_JS, /setMaintenanceWorkspaceTab\([^;]+\{ focus: true \}\)/);
});
