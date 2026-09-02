import test from "node:test";
import assert from "node:assert/strict";
import { formatTimestamp } from "../src/time-format.js";

test("date-only observations preserve day precision in every timezone", () => {
  for (const locale of ["zh-CN", "en-US"]) {
    for (const timeZone of ["Asia/Shanghai", "UTC", "America/Los_Angeles"]) {
      assert.equal(formatTimestamp("2026-09-02", locale, {timeZone}), "2026-09-02");
      assert.equal(formatTimestamp("2024-02-29", locale, {timeZone}), "2024-02-29");
    }
  }
});

test("actual instants keep timezone conversion and explicit offsets", () => {
  const options = {timeZone:"Asia/Shanghai", hour12:false};
  const expected = new Date("2026-09-02T14:48:35.809Z").toLocaleString("zh-CN", options);
  assert.equal(formatTimestamp("2026-09-02T14:48:35.809Z", "zh-CN", options), expected);
  assert.equal(formatTimestamp("2026-09-02T22:48:35.809+08:00", "zh-CN", options), expected);
  assert.match(expected, /22:48:35/);
});

test("missing and unparseable timestamps are not invented", () => {
  for (const value of [null, undefined, ""]) assert.equal(formatTimestamp(value), "-");
  assert.equal(formatTimestamp("unknown"), "unknown");
});
