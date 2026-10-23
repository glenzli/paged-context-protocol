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
memories** or **Read and contribute**, and choose **All scopes** or **Selected scopes**. Read and All scopes are
the defaults. All scopes applies that mode to current and future Scopes;
Selected scopes shows a searchable checklist. Existing registrations keep their
previous grants; the new default never upgrades them.
**Register agent** creates an independent identity awaiting approval; review
its displayed permissions and choose **Authorize agent** before using it.
Candidate/activity opt-ins and new-project registration retain their separate
controls.

Choose **Target client** before or after registration. An approved agent offers
ready-to-use configuration, client-specific installation steps, an official guide,
and copy/download actions. This is an export choice, not a new permission grant;
the browser remembers it per local profile. Each independent agent should have
its own registration. The process command and enrollment-file reference are filled
in. Credentials stay in the managed local file and never reach the browser. These
exports work on the computer and user account that can access that file.

| Client | Export and installation |
| --- | --- |
| Claude Code | `mcpServers` JSON plus a safely quoted `claude mcp add-json --scope user` command; project `.mcp.json` is an alternative. |
| Claude Desktop | `mcpServers` JSON; open **Settings → Developer → Edit Config**. |
| Codex | TOML tables for `~/.codex/config.toml`. |
| Cursor | `mcpServers` JSON for user or project `mcp.json`. |
| VS Code / GitHub Copilot | `servers` JSON with `type: stdio` for MCP User Configuration or `.vscode/mcp.json`. |
| Gemini CLI | `mcpServers` JSON merged into `settings.json`. |
| Antigravity | `mcpServers` JSON; open the installed version's MCP config from its settings. |
| DeepSeek Harness (DSH) | A complete Cordis `insert` overlay for `@deepseek-ai/dsh-mcp-client`, plus `dsh web --patch` instructions. |
| OpenCode 1.x / 2.x | Separate exports for `mcp.<name>` and `mcp.servers.<name>`, using command arrays and `environment`. |
| Cline / Roo Code | `mcpServers` JSON merged from the extension's MCP configuration editor. |
| Windsurf / Cascade | `mcpServers` JSON for Cascade; the newer Devin Local agent has separate configuration. |
| Generic Agent | Generic `mcpServers` JSON and a standard STDIO MCP client SDK workflow. |

The Console does not install clients, edit their settings, or change their tool
approval defaults. Merge entries without replacing existing settings. Claude's
install command is for a local POSIX shell. DSH's command runs from the directory
containing its downloaded YAML. Its server namespace is a lossless base-36 UUID
to fit DSH's 32-character limit; the PCP Principal and credential stay unchanged.

**Verify in the target agent** provides a copyable read-only prompt with the
expected Principal and scopes. A successful config export or Console session is
not proof that a target agent connected: check that agent's real `pcp_whoami`,
`pcp_list_scopes`, and read/search results. **PCP usage guidance** supplies optional
persistent instructions for focused recall; write guidance follows the granted
mode and keeps candidate/activity opt-ins separate.

`src/local-client-config.js` owns the client catalog, formats, installation text,
verification prompts and their Chinese translations. It projects the existing
approved launch specification without changing the backend API or stored profiles.
The catalog links the official format sources checked on 2026-10-23. A catalog
entry denotes an available export, not an assertion that every client/version has
been run. Regenerate against the relevant official guide when a client changes.

Configuration validation combines `frontend/local-client-config.test.mjs` and
`frontend/local-client-setup.test.mjs` with `scripts/test_console_client_exports.py`.
The integration script uses real JSON/TOML/YAML parsers and isolated PCP processes,
reads a synthetic Page and checks read-only denial for every export.
`scripts/test_console_all_scopes.py` also keeps real MCP processes connected
while new project Scopes are created, verifies read/contribute modes before and
after a Runtime restart, proves selected legacy grants stay bounded, and checks
that revocation invalidates running all-scope connections. Optional
installed-client parser checks use fresh homes and no model calls. They are
reported separately from an end-to-end conversation in that branded client.

Local profiles remain listed after reload, including revoked registrations.
**Ready to connect** means the configuration and grant are available; it does
not prove an agent has connected. Runtime session timestamps also include
Console activity. Unknown writes freeze further permission changes and expose
a read-only **Check current state** action. An authoritative missing registration
returns to the retained draft; it never silently creates another identity or
approves a request. If a fresh check confirms an interrupted registration can
resume, **Resume registration** explicitly retries that original identity and
immutable grant. It does not approve it. The immutable request includes the
all/selected scope policy; all-scope grants are summarized as **All scopes**
instead of enumerating the Store. All-scope contribution grants ingest, without
revision, administration or cross-scope derivation authority. Draft cancellation, Escape and browser
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
