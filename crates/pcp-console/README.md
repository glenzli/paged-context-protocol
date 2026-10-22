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
the shared mutation helper. Failed actions stay visible inside the dialog.
An unknown outcome freezes further permission actions until the operator
explicitly refreshes and checks the current permissions; requests are not replayed.

### Local agent setup

In **Client access**, choose **Add local agent**, enter a name, select **Read
memories** or **Read and contribute**, and choose each permitted Scope. Read is
the default and no Scope is selected initially. The Scope list supports search.
**Register agent** creates an independent identity awaiting approval; review
its displayed permissions and choose **Authorize agent** before using it.
Candidate/activity opt-ins and new-project registration retain their separate
controls.

An approved local agent offers a complete generic MCP JSON configuration to
copy or download. Its command and local enrollment-file reference are already
filled in. Credentials remain in the managed local file and are never sent to
the browser. Each configuration uses its own server key; the Console does not
edit another application's files or install software. Keep this configuration
on the computer where it was generated. The MCP format must be accepted by the
agent's own configuration UI.

Local profiles remain listed after reload, including revoked registrations.
**Ready to connect** means the configuration and grant are available; it does
not prove an agent has connected. Runtime session timestamps also include
Console activity. Unknown writes freeze further permission changes and expose
a read-only **Check current state** action. An authoritative missing registration
returns to the retained draft; it never silently creates another identity or
approves a request. If a fresh check confirms an interrupted registration can
resume, **Resume registration** explicitly retries that original identity and
immutable grant. It does not approve it. Draft cancellation, Escape and browser
close protect unsubmitted work. `src/local-client-setup.js` owns this workflow and configuration
recovery; `src/client-access-view.js` combines local profiles with existing
permissions without duplicating their rows.

The overview starts with human review decisions, pending client approvals and
blocked work, each linking to its existing editor. Automatic waits and completed
history are excluded. Each source retains its own loading, unavailable and retry
state. `src/overview-actions.js` owns these read-only checks; the workflow guide
is a closed disclosure beneath them.

The overview can filter projects and Scopes by name. Query activity diagnostics
follow the results in a closed disclosure. Query results retain the executed
query, Scope, method and timestamp. Changing the form marks retained results as
belonging to earlier conditions. Runtime restart is available from
the Runtime page, with its existing confirmation. Errors remain visible until
dismissed so a failed action can be inspected. Maintenance diagnostics separate
budget admission waits from failed calls and show current allowance alongside
retained waits; an old wait does not establish current exhaustion.

Settings opens the local General panel immediately. Automatic maintenance
loads its remote configuration and model catalog on entry, with local retry.
`src/maintenance-settings-session.js` owns the settings draft, stale-response
checks and save recovery. Polls and language changes preserve edited controls.
Close and Escape confirm discarding changed settings; an unknown save can be
closed explicitly and checked later while the Console page remains open.
Its draft stays frozen until a read-only check resolves the current state.

## Activity and candidate evidence

Activity groups topics by project/Scope, with the most recently updated groups
first. Publishers can provide a readable title, status, next step and an explicit
parent topic. Legacy cards retain their complete summary without an inferred
completion state. Project filtering and parent/child presentation are owned by
`src/context-activity.js`.

Each Context Hub tab reads its own snapshot: `GET /api/context-hub/activity`,
`/policies` or `/inbox`. Activity does not download candidate history. Candidate
and synthesis lists contain summaries; expanding evidence or starting a review
fetches `/candidate/{id}` or `/synthesis/{id}`. Full evidence and exact prior
requests remain available for review and interrupted-submission recovery.
`src/context-hub.js` owns these requests, version checks and unsubmitted drafts.
Detail tokens invalidate cached evidence when background review, receipts or
undo records change without incrementing the reviewable version.
The complete `GET /api/context-hub` response remains available for existing clients.

## Direct page actions

Open a Page and use **Edit content** to update plain text or Markdown. Saving
publishes a new Revision immediately, without a maintenance review or a reason
form. Page identity, sources and facets stay unchanged. The editor reads the
complete available payload and refuses truncated or structured Pack content.
Concurrent changes reject the save and leave the draft in the editor.
Page edits, candidate/synthesis edits and maintenance settings show unsaved
status and confirm editor cancellation. Browser refresh or close protects their
drafts and in-flight submissions. Theme and language apply immediately.

**Delete Page** asks for one confirmation. It publishes a tombstone for that
Page and retracts its incident relations; it does not delete other Pages or
run the derivation-retraction cascade. Deleted Pages leave current retrieval.
Historical Revisions remain available for audit; this is not physical erasure.

Both actions require the local Console mutation header and Runtime-authorized
permissions. Ordinary MCP model clients cannot use the repair or deletion API.

Frontend checks: `npm run test:web`. Backend checks: `cargo test -p pcp-console`.
