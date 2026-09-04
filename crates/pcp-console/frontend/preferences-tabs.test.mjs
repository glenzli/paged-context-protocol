import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const INDEX_HTML = readFileSync(new URL("../src/index.html", import.meta.url), "utf8");
const STYLES_CSS = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

test("settings tabs control one panel each", () => {
  assert.match(INDEX_HTML, /id="preferences-general-tab"[^>]+aria-controls="preferences-general-panel"/);
  assert.match(INDEX_HTML, /id="maintenance-settings-tab"[^>]+aria-controls="maintenance-settings-section"/);
  assert.equal((INDEX_HTML.match(/data-preferences-panel="general"/g) || []).length, 1);
  assert.equal((INDEX_HTML.match(/data-preferences-panel="maintenance"/g) || []).length, 1);
  assert.match(INDEX_HTML, /id="preferences-general-panel"[^>]+role="tabpanel"[^>]+aria-labelledby="preferences-general-tab"/);
  assert.match(INDEX_HTML, /id="maintenance-settings-section"[^>]+role="tabpanel"[^>]+aria-labelledby="maintenance-settings-tab"/);
});

test("settings panels are not also display-forcing content sections", () => {
  assert.doesNotMatch(INDEX_HTML, /class="[^"]*preferences-section preferences-panel/);
  assert.match(STYLES_CSS, /\.preferences-panel\s*\{[^}]*display:\s*none/);
  assert.match(STYLES_CSS, /\.preferences-panel\.active\s*\{[^}]*display:\s*grid/);
});
