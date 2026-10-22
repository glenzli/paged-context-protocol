import { maintenanceFailureKind, maintenanceFailureLabel } from "../src/maintenance-diagnostics.js";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const APP_JS = readFileSync(new URL("../src/app.js", import.meta.url), "utf8");
const INDEX_HTML = readFileSync(new URL("../src/index.html", import.meta.url), "utf8");

function diagnosticView() {
  const document = { activeElement: null };
  class Node {
    constructor(tag, text = "") { this.tag = tag; this.textContent = text; this.children = []; this.dataset = {}; this.style = {}; this.open = false; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
    contains(node) { return this === node || this.children.some((child) => child.contains(node)); }
    querySelectorAll(selector) {
      const match = (node) => selector === "summary" ? node.tag === "summary" : node.tag === "details" && node.open;
      return this.children.flatMap((child) => [...(match(child) ? [child] : []), ...child.querySelectorAll(selector)]);
    }
    focus() { document.activeElement = this; }
  }
  const context = { maintenanceFailureKind, maintenanceFailureLabel, document, currentLanguage: "zh", element: (tag, cls, text) => new Node(tag, text), formatTime: (value) => value };
  const start = APP_JS.indexOf("function groupMaintenanceJobIssues(");
  const end = APP_JS.indexOf("function populateMaintenanceSettings(", start);
  assert.ok(start >= 0 && end > start);
  runInNewContext(APP_JS.slice(start, end), context);
  return { context, container: new Node("div") };
}

test("diagnostics group repeated causes and keep raw errors and revisions behind disclosures", () => {
  const { context, container } = diagnosticView();
  const jobIssues = Array.from({ length: 24 }, (_, i) => ({ operation: "select_relation", reason: "serverOverloaded private failure detail", sourceRevisionIds: [`rev_${i}`], attempts: 1, retryAt: "later" }));
  context.renderMaintenanceJobIssues(container, { jobIssues });
  assert.equal(container.children.length, 1);
  const active = container.children[0];
  assert.equal(active.open, false);
  assert.match(active.children[0].textContent, /等待重试 24 项.*上游繁忙 24/);
  assert.doesNotMatch(active.children[0].textContent, /rev_|private failure/);
  const group = active.children[1];
  assert.equal(group.open, false);
  assert.equal(group.children.length, 25);
  const issue = group.children[1];
  assert.equal(issue.open, false);
  assert.match(issue.children[1].textContent, /private failure detail/);
  assert.equal(issue.children[3].textContent, "rev_0");
});

test("diagnostic polling preserves open disclosures and keyboard focus, and history stays separate", () => {
  const { context, container } = diagnosticView();
  const issue = { operation: "topic", reason: "deadline elapsed", sourceRevisionIds: ["rev_1"], attempts: 2, retryAt: "later" };
  const status = { jobIssues: [issue], jobIssueHistory: [{ ...issue, closedAt: "earlier", disposition: "sources_changed" }], archivedJobIssueCount: 150 };
  context.renderMaintenanceJobIssues(container, status);
  const active = container.children[0];
  active.open = true;
  active.children[0].focus();
  context.renderMaintenanceJobIssues(container, status);
  assert.equal(container.children[0], active);
  context.renderMaintenanceJobIssues(container, { ...status, jobIssues: [{ ...issue, deferredChecks: 1 }] });
  assert.equal(container.children[0].open, true);
  assert.equal(context.document.activeElement, container.children[0].children[0]);
  assert.equal(container.children[1].open, false);
  assert.match(container.children[1].children[0].textContent, /最近 1 项／累计 150 项/);
  context.renderMaintenanceJobIssues(container, { jobIssues: [], jobIssueHistory: [] });
  assert.equal(container.hidden, true);
});

test("exhausted automatic failures wait for new evidence instead of showing another retry", () => {
  const { context, container } = diagnosticView();
  context.renderMaintenanceJobIssues(container, { jobIssues: [{ operation: "topic", reason: "deadline elapsed", sourceRevisionIds: ["rev_1"], attempts: 3, retryAt: "obsolete", automaticRetryExhausted: true }] });
  const active = container.children[0];
  assert.match(active.children[0].textContent, /等待重试 0 项.*等待新证据 1 项/);
  assert.match(active.children[1].children[1].children[2].textContent, /自动重试已停止，等待新证据/);
  assert.doesNotMatch(active.children[1].children[1].children[2].textContent, /obsolete|下次重试/);
});

test("budget waits are deduplicated separately and current allowance does not imply retry of unknown execution", () => {
  const { context, container } = diagnosticView();
  const reason = "Waiting for maintenance worker: PCP total maintenance budget exhausted; waiting for rolling budget release";
  const jobIssues = Array.from({ length: 22 }, (_, i) => ({ operation:"assess_archive", reason, failureKind:"worker_failed", sourceRevisionIds:[`rev_${i}`], attempts:1, retryAt:"later" }));
  jobIssues.push({operation:"topic",reason:"maintenance response wait timed out; outcome unknown",failureKind:"response_timeout",sourceRevisionIds:["rev_unknown"],attempts:3,automaticRetryExhausted:true});
  const budget = {status:{total:{remainingCalls:215,maxCalls:360,remainingTokens:5635209}}};
  context.renderMaintenanceJobIssues(container, { jobIssues }, budget);
  assert.equal(container.children.length, 2);
  const waiting = container.children[0], failures = container.children[1];
  assert.match(waiting.children[0].textContent,/预算等待记录 22/);
  assert.match(waiting.children[2].textContent,/当前剩余 215\/360 次调用.*5,635,209/);
  assert.match(waiting.children[3].textContent,/当前仍有额度.*不代表此刻耗尽/);
  const group = waiting.children[1];
  assert.equal(group.children.filter(node=>node.textContent === reason).length,1);
  assert.match(group.children[2].children[0].textContent,/预算准入等待/);
  assert.doesNotMatch(group.children[2].children[0].textContent,/调用尝试/);
  assert.match(failures.children[0].textContent,/等待新证据 1 项.*结果未确认/);
  assert.match(failures.children[1].children[1].children[2].textContent,/自动重试已停止/);
  waiting.open = true; waiting.children[0].focus();
  context.renderMaintenanceJobIssues(container, { jobIssues }, {status:{total:{remainingCalls:0,maxCalls:360,remainingTokens:0}}});
  assert.equal(container.children[0].open,true);
  assert.equal(context.document.activeElement,container.children[0].children[0]);
  assert.match(container.children[0].children[3].textContent,/当前额度等待释放/);
});

test("missing current budget remains unknown and stopped attempts retain their evidence gate", () => {
  const { context, container } = diagnosticView();
  context.renderMaintenanceJobIssues(container, {jobIssues:[{
    operation:"topic",failureKind:"waiting_budget",reason:"local budget denial",sourceRevisionIds:["rev_wait"],
    attempts:3,automaticRetryExhausted:true,retryAt:"must-not-retry",
  }]});
  assert.match(container.children[0].children[2].textContent,/当前预算未读取/);
  const item = container.children[0].children[1].children[2];
  assert.match(item.children[1].textContent,/等待新证据/);
  assert.doesNotMatch(item.children[1].textContent,/must-not-retry|下次准入检查/);
});

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
