import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { iconDefinition, iconNames } from "../src/ui-icons.js";

const INDEX_HTML = readFileSync(new URL("../src/index.html", import.meta.url), "utf8");
const STYLES_CSS = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

const REQUIRED_ICONS = [
  "restart", "refresh", "run", "search", "scan", "rescan", "analyze", "retry",
  "pack", "summary", "relation", "reconciliation", "topic", "archive", "accept", "reject",
  "defer", "suppress", "undo", "apply", "end",
];

test("semantic controls have one registered icon definition", () => {
  const names = new Set(iconNames());
  for (const name of REQUIRED_ICONS) {
    assert.equal(names.has(name), true, `missing icon: ${name}`);
    assert.ok(iconDefinition(name).length > 0, `empty icon: ${name}`);
  }
});

test("unknown icons fail closed at the registry boundary", () => {
  assert.equal(iconDefinition("not-a-real-control"), null);
});

test("restart combines a circular arrow with a power mark and stays distinct from refresh", () => {
  assert.notDeepEqual(iconDefinition("restart"), iconDefinition("refresh"));
  assert.equal(iconDefinition("restart").length, 4);
  assert.equal(iconDefinition("refresh").length, 2);
  assert.equal(iconDefinition("restart").includes("M12 8.5v3.3"), true);
});

test("analyze uses a report chart instead of decorative sparkles", () => {
  const paths = iconDefinition("analyze");
  assert.equal(paths[0], "M5 3h9l5 5v13H5z");
  assert.equal(paths.some((path) => path.includes("17v-6")), true);
  assert.equal(paths.some((path) => path.includes("3.3")), false);
});

test("rescan places a recognizable repeat arrow inside the search lens", () => {
  const paths = iconDefinition("rescan");
  assert.equal(paths[0], "M15.5 15.5 20 20");
  assert.equal(paths[1], "M17 9a8 8 0 1 1-16 0 8 8 0 0 1 16 0");
  assert.equal(paths.includes("M12.5 5.5V9H9"), true);
  assert.notDeepEqual(paths, iconDefinition("scan"));
});

test("maintenance feature entry controls are icon-only and retain accessible labels", () => {
  for (const id of ["maintenance-start", "maintenance-manual-start", "maintenance-start-new", "archive-start", "archive-start-new"]) {
    const match = INDEX_HTML.match(new RegExp(`<button\\s+id="${id}"([^>]*)>([\\s\\S]*?)</button>`));
    assert.ok(match, `missing entry control: ${id}`);
    assert.match(match[1], /maintenance-entry-icon-button/);
    assert.match(match[1], /data-icon="[^"]+"/);
    assert.match(match[1], /aria-label="[^"]+"/);
    assert.match(match[1], /data-i18n-aria-label="[^"]+"/);
    assert.equal(match[2].trim(), "", `${id} should not render a text label`);
  }
});

test("maintenance workflow action rows use one icon-only control language", () => {
  for (const id of [
    "maintenance-primary", "maintenance-skip", "maintenance-retry-failed", "maintenance-rescan", "maintenance-cancel",
    "archive-analyze", "archive-apply", "archive-retry-failed", "archive-rescan", "archive-finish",
  ]) {
    const match = INDEX_HTML.match(new RegExp(`<button\\s+id="${id}"([^>]*)>([\\s\\S]*?)</button>`));
    assert.ok(match, `missing maintenance action: ${id}`);
    assert.match(match[1], /maintenance-action-icon-button/);
    assert.match(match[1], /aria-label="[^"]+"/);
    assert.equal(match[2].trim(), "", `${id} should not mix icon and text content`);
  }
});

test("maintenance action controls override the compact button width and remain square", () => {
  const rule = STYLES_CSS.match(/\.compact-icon-button\.maintenance-action-icon-button\s*\{([^}]*)\}/);
  assert.ok(rule);
  assert.match(rule[1], /width:\s*42px/);
  assert.match(rule[1], /min-width:\s*42px/);
  assert.match(rule[1], /height:\s*42px/);
  assert.match(rule[1], /min-height:\s*42px/);
});

test("the Context Pack query submit control is an accessible icon-only search action", () => {
  const match = INDEX_HTML.match(/<button\s+id="context-query-submit"([^>]*)>([\s\S]*?)<\/button>/);
  assert.ok(match);
  assert.match(match[1], /query-submit-icon-button/);
  assert.match(match[1], /data-icon="search"/);
  assert.match(match[1], /aria-label="Build context pack"/);
  assert.equal(match[2].trim(), "");
});
