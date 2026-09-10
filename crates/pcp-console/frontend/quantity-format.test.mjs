import test from "node:test";
import assert from "node:assert/strict";
import { compactQuantity, exactQuantity } from "../src/quantity-format.js";

test("Chinese quantities use 万 and 亿, English quantities use k/m/b", () => {
  for (const [value, zh, en] of [[0,"0","0"],[961,"961","961"],[12959,"1.3万","12.96k"],[12187476,"1218.75万","12.19m"],[5639347,"563.93万","5.64m"],[100000000,"1亿","100m"],[2500000000,"25亿","2.5b"]]) {
    assert.equal(compactQuantity(value,"zh-CN"),zh);
    assert.equal(compactQuantity(value,"en-US"),en);
  }
});
test("rounding promotes units and preserves missing values and exact details", () => {
  assert.equal(compactQuantity(999999,"en-US"),"1m");
  assert.equal(compactQuantity(99999999,"zh-CN"),"1亿");
  assert.equal(compactQuantity(10000,"zh-CN"),"1万");
  assert.equal(compactQuantity(9999,"zh-CN"),"9,999");
  assert.equal(compactQuantity(null),"—");
  assert.equal(compactQuantity(NaN),"—");
  assert.equal(exactQuantity(12187476,"zh-CN"),"12,187,476");
});
