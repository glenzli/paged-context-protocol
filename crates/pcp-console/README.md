# PCP Console

The local Console manages the connected PCP Store. A managed Console uses a
Store-wide operator endpoint; tenant and model-client grants stay separate.

## Direct page actions

Open a Page and use **Edit content** to update plain text or Markdown. Saving
publishes a new Revision immediately, without a maintenance review or a reason
form. Page identity, sources and facets stay unchanged. The editor reads the
complete available payload and refuses truncated or structured Pack content.
Concurrent changes reject the save and leave the draft in the editor.

**Delete Page** asks for one confirmation. It publishes a tombstone for that
Page and retracts its incident relations; it does not delete other Pages or
run the derivation-retraction cascade. Deleted Pages leave current retrieval.
Historical Revisions remain available for audit; this is not physical erasure.

Both actions require the local Console mutation header and Runtime-authorized
permissions. Ordinary MCP model clients cannot use the repair or deletion API.

Frontend checks: `npm run test:web`. Backend checks: `cargo test -p pcp-console`.
