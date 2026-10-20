# PCP Console

The local Console manages the connected PCP Store. A managed Console uses a
Store-wide operator endpoint; tenant and model-client grants stay separate.

## Information and navigation

Access lists show the client, operation and result. Recorded Scope names,
session identifiers and internal calls remain in request details. A request
without recorded storage operations shows its session permissions there;
these are not evidence that every authorized project was read. Operation
filters use readable labels, retain exact identifiers in requests, and offer
one action to clear filters. Loading another page preserves open request details.

**Client access** groups each client's permission policy, named project Scopes,
and connection identifiers. Candidate/activity permissions link to their existing
editor. Unchanged background polls preserve the open disclosure and keyboard
focus; revocation asks for confirmation. The snapshot and enrollment actions
are owned by `src/client-access-view.js`; the application wires navigation and
the shared mutation/error helpers.

The overview can filter projects and Scopes by name. Query activity diagnostics
follow the results in a closed disclosure. Runtime restart is available from
the Runtime page, with its existing confirmation. Errors remain visible until
dismissed so a failed action can be inspected.

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
