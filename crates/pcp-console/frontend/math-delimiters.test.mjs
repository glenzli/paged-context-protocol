import assert from "node:assert/strict";
import test from "node:test";

import { protectMath } from "./math-delimiters.mjs";

test("protects common inline and display math delimiters", () => {
  const source = String.raw`
\[
O_c = O+\delta_c
\]

Inline: \(g_t\), $J_t$, and:

$$
\Delta J_t \approx \langle g_t,\delta_c\rangle
$$
`;

  const result = protectMath(source);

  assert.deepEqual(result.math, [
    { expression: String.raw`O_c = O+\delta_c`, display: true },
    {
      expression: String.raw`\Delta J_t \approx \langle g_t,\delta_c\rangle`,
      display: true,
    },
    { expression: "g_t", display: false },
    { expression: "J_t", display: false },
  ]);
  assert.match(result.protectedMarkdown, /data-pcp-math="0"/);
  assert.match(result.protectedMarkdown, /data-pcp-math="3"/);
});

test("leaves escaped delimiters and ordinary brackets unchanged", () => {
  const source = String.raw`Keep \\[literal\\], \[math\], [ordinary], and \$5.`;
  const result = protectMath(source);

  assert.deepEqual(result.math, [{ expression: "math", display: true }]);
  assert.match(result.protectedMarkdown, /\\\\\[literal\\\\\]/);
  assert.match(result.protectedMarkdown, /\[ordinary\]/);
  assert.match(result.protectedMarkdown, /\\\$5/);
});

test("does not treat multiline single-dollar content as inline math", () => {
  const source = "$first line\nsecond line$";
  const result = protectMath(source);

  assert.deepEqual(result.math, []);
  assert.equal(result.protectedMarkdown, source);
});
