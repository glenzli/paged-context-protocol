import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pageListPreview, pageCount, pageJump, pageRoleBadge, appendPageFilters, pageTimeFields, pageBrowseOrder } from "../src/page-list.js";

test("stored time and source observation time remain distinct", () => {
  const createdAt = "2026-09-02T14:48:35Z";
  const updatedAt = "2026-09-03T10:00:00Z";
  assert.deepEqual(pageTimeFields({createdAt,updatedAt,observedAt:"2026-09-02"}), [
    ["Updated",updatedAt], ["Observed","2026-09-02"],
  ]);
  assert.deepEqual(pageTimeFields({createdAt}), [["Stored",createdAt]]);
  assert.deepEqual(pageTimeFields({createdAt,updatedAt}), [["Updated",updatedAt]]);
});

test("default update ordering and explicit observation ordering are independent", () => {
  assert.equal(pageBrowseOrder("updated",true),"updated");
  assert.equal(pageBrowseOrder("updated",false),"least_recently_updated");
  assert.equal(pageBrowseOrder("observed",true),"recent");
  assert.equal(pageBrowseOrder("observed",false),"oldest");
  assert.equal(pageBrowseOrder("connections",true),"most_connected");
  assert.equal(pageBrowseOrder("connections",false),"least_connected");
  assert.equal(pageBrowseOrder(undefined,true),"updated");
});

test("content badges use structural metadata, not suggestive page kinds", () => {
  for (const hit of [{kind:"topic_summary"}, {kind:"summary_projection"}, {facets:{role:"condensed"}}, {summaryRevisionId:"rev_summary"}, {contentRole:"other"}]) {
    assert.equal(pageRoleBadge(hit), null);
  }
  assert.equal(pageRoleBadge({contentRole:"condensed",kind:"document"}).label,"Condensed summary");
  assert.equal(pageRoleBadge({contentRole:"covered_source"}).label,"Summarized source");
});

test("structural filters combine with scope, query and direct page number", () => {
  const params = new URLSearchParams({scope:"user:self",q:"PCP",page:"3"});
  appendPageFilters(params,{role:"condensed",withSummary:true});
  assert.deepEqual(Object.fromEntries(params),{scope:"user:self",q:"PCP",page:"3",role:"condensed",withSummary:"true"});
  assert.equal(appendPageFilters(new URLSearchParams(),{role:"",withSummary:false}).toString(), "");
  assert.equal(appendPageFilters(new URLSearchParams(),{role:"invented"}).toString(), "");
});

const markdown = (title) => ({facets:{title},previewPayload:{mediaType:"text/markdown"}});

test("explicit title is separate and a repeated Markdown heading is removed", () => {
  assert.deepEqual(pageListPreview(markdown("PCP 的定位"), "# PCP 的定位\n\n正文说明"), {title:"PCP 的定位",excerpt:"正文说明"});
  assert.deepEqual(pageListPreview(markdown("PCP 的定位"), "PCP 的定位\n\n正文说明"), {title:"PCP 的定位",excerpt:"正文说明"});
  assert.deepEqual(pageListPreview(markdown("PCP"), "PCP Runtime 的正文不是重复标题"), {title:"PCP",excerpt:"PCP Runtime 的正文不是重复标题"});
});

test("untitled text stays an ordinary excerpt and Markdown can supply a title", () => {
  assert.deepEqual(pageListPreview({}, "一段普通内容\n下一行"), {title:"",excerpt:"一段普通内容 下一行"});
  assert.deepEqual(pageListPreview(markdown(), "## 标题 ##\r\n\r\n正文"), {title:"标题",excerpt:"正文"});
  assert.deepEqual(pageListPreview(markdown("显式标题"), "# 其他标题\n正文"), {title:"显式标题",excerpt:"# 其他标题 正文"});
  assert.deepEqual(pageListPreview({facets:{title:{bad:1}}}, "内容"), {title:"",excerpt:"内容"});
  assert.deepEqual(pageListPreview(markdown("只有标题"), "# 只有标题"), {title:"只有标题",excerpt:""});
});

test("tenant JSON and conversation excerpts are not interpreted as Markdown headings", () => {
  assert.deepEqual(pageListPreview({previewPayload:{mediaType:"application/json"}}, "# plain JSON string"), {title:"",excerpt:"# plain JSON string"});
  const unsafe = '<img src=x onerror=alert(1)>';
  assert.equal(pageListPreview(markdown(unsafe), "正文").title, unsafe); // Render using textContent, never innerHTML.
});

test("page jumps accept only bounded whole page numbers", () => {
  assert.equal(pageCount(81,20),5);
  assert.equal(pageCount(0,20),1);
  assert.equal(pageJump("5",81,20),5);
  assert.equal(pageJump(" 2 ",81,20),2);
  for (const value of ["", "0", "-1", "1.5", "1e1", "6", "9007199254740993"]) assert.equal(pageJump(value,81,20),null);
});

test("list markup provides native accessible page-number submission", () => {
  const html = readFileSync(new URL("../src/index.html",import.meta.url),"utf8");
  assert.match(html, /id="pages-current"[^>]+type="number"[^>]+min="1"[^>]+required/);
  assert.match(html, /id="pages-jump-submit"[^>]+type="submit"[^>]+aria-label="Jump to page"/);
  const app = readFileSync(new URL("../src/app.js",import.meta.url),"utf8");
  assert.match(app, /page: String\(page\)/);
  assert.doesNotMatch(app, /state\.pages\.cursors/);
});
