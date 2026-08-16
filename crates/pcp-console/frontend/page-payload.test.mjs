import assert from "node:assert/strict";
import test from "node:test";

import { describePagePayload, pagePayloadPreviewText } from "./page-payload.mjs";

test("presents an external signal as semantic fields", () => {
  const source = JSON.stringify({
    title: "Inference prices and infrastructure costs diverge",
    summary: "Unit prices are falling while capital costs remain high.",
    content: "This tension is worth monitoring.",
    event_at: "2026-07-24",
    qualification_note: "Source links were not independently verified.",
    review_reason: "Relevant external signal.",
  });

  assert.deepEqual(
    describePagePayload(source, "application/vnd.symbiont.external-signal+json"),
    {
      type: "external_signal",
      title: "Inference prices and infrastructure costs diverge",
      summary: "Unit prices are falling while capital costs remain high.",
      content: "This tension is worth monitoring.",
      eventAt: "2026-07-24",
      qualificationNote: "Source links were not independently verified.",
      reviewReason: "Relevant external signal.",
    },
  );
  assert.equal(
    pagePayloadPreviewText(source, "application/vnd.symbiont.external-signal+json"),
    "Inference prices and infrastructure costs diverge — This tension is worth monitoring.",
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

test("summarizes image metadata without inventing an asset URL", () => {
  const source = JSON.stringify({
    filename: "image.png",
    mimeType: "image/png",
    width: 3224,
    height: 1460,
    byteSize: 316702,
  });

  assert.equal(
    pagePayloadPreviewText(source, "application/vnd.symbiont.image+json"),
    "image.png — 3224 × 1460 — image/png",
  );
});

test("keeps malformed or unknown content available as a safe fallback", () => {
  assert.deepEqual(
    describePagePayload('{"content":"truncated', "application/example+json"),
    { type: "raw", content: '{"content":"truncated' },
  );
  assert.equal(pagePayloadPreviewText("plain text", "text/plain"), "plain text");
});
