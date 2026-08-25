import assert from "node:assert/strict";
import test from "node:test";

import { describePagePayload, pagePayloadPreviewText } from "./page-payload.mjs";

test("keeps tenant-specific external signals as generic JSON", () => {
  const source = JSON.stringify({
    title: "Inference prices and infrastructure costs diverge",
    summary: "Unit prices are falling while capital costs remain high.",
    content: "This tension is worth monitoring.",
    event_at: "2026-07-24",
    qualification_note: "Source links were not independently verified.",
    review_reason: "Relevant external signal.",
  });

  const presentation = describePagePayload(
    source,
    "application/vnd.symbiont.external-signal+json",
  );
  assert.equal(presentation.type, "json");
  assert.deepEqual(presentation.value, JSON.parse(source));
  assert.equal(
    pagePayloadPreviewText(source, "application/vnd.symbiont.external-signal+json"),
    "Inference prices and infrastructure costs diverge",
  );
});

test("extracts readable conversation text from a packed Page", () => {
  const source = JSON.stringify({
    entries: [
      {
        pageId: "pg-user",
        createdAt: "2026-08-15T06:48:05Z",
        facets: { role: "user" },
        payload: { mediaType: "text/markdown", content: "How should the protocol stay small?" },
      },
      {
        pageId: "pg-assistant",
        createdAt: "2026-08-15T06:48:42Z",
        facets: { role: "assistant" },
        payload: { mediaType: "text/markdown", content: "Keep the invariant core narrow." },
      },
    ],
  });

  const presentation = describePagePayload(source, "application/vnd.pcp.packed-page+json");
  assert.equal(presentation.type, "packed_page");
  assert.equal(presentation.entries.length, 2);
  assert.equal(
    pagePayloadPreviewText(source, "application/vnd.pcp.packed-page+json"),
    "User — How should the protocol stay small? · Assistant — Keep the invariant core narrow.",
  );
});

test("keeps tenant-specific image payloads as generic JSON", () => {
  const source = JSON.stringify({
    filename: "image.png",
    mimeType: "image/png",
    width: 3224,
    height: 1460,
    byteSize: 316702,
  });

  const presentation = describePagePayload(source, "application/vnd.symbiont.image+json");
  assert.equal(presentation.type, "json");
  assert.deepEqual(presentation.value, JSON.parse(source));
});

test("keeps malformed or unknown content available as a safe fallback", () => {
  assert.deepEqual(
    describePagePayload('{"content":"truncated', "application/example+json"),
    { type: "raw", content: '{"content":"truncated' },
  );
  assert.equal(pagePayloadPreviewText("plain text", "text/plain"), "plain text");
});
