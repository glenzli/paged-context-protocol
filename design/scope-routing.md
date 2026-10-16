# Shared-client write Scope routing

Scope is an authorization and maintenance boundary. The writer chooses it from the
subject's owner; the authenticated Principal records the client and SourceRefs point
to actual evidence. Neither client identity nor a source label determines ownership.

| Subject | Destination |
| --- | --- |
| User preference, constraint, or decision that applies across projects | `user:self` |
| Project-specific design, product rule, finding, or useful outcome | The existing Scope for that project, or an approved `project:<slug>` Scope |
| Project checkpoint or uncertain project evidence | Activity or candidate in the same project Scope |
| Lesson established as applicable across projects | A separately reviewed user-level account with precise source Revisions and limits |

Useful attempts and observations remain in the project Scope where their conditions
and evidence live. `reusable_outcome` and structured experience evidence describe
their content; a global experience Scope would separate them from the same-Scope
candidate organizer. Existing named project Scopes, such as `symbiont-d`, may be
reused rather than duplicated under `project:`.

## Project registration

At the first meaningful project capture, candidate, or activity, call
`pcp_ensure_project_scope` with a stable `projectKey` and `displayName`.
Use explicit project metadata or a verified repository identity such as
`github.com/owner/repository`, without credentials, branch names, or worktree paths.
Reuse the same key and receipt across branches and topics. Several topics inside
one project do not turn a project rule into a cross-project user preference.
Do not create empty Scopes on every project visit.

In Console, an operator enables automatic project registration for an approved
contributing client. Existing registrations default to disabled. Runtime persists
a unique key-to-Scope binding and a provisioning intent before creating the Scope,
then grants contribution and returns a freshly authenticated session. Identical
and concurrent requests reuse the binding; interrupted provisioning can resume.
No Scope administration permission is granted to the model client. A client cannot
claim another registration's existing project without its current write grant.
`existingScope` explicitly adopts an already writable non-user Scope instead of
creating another one. Disabling registration prevents new bindings/grants while
retaining existing project access; revoking the client removes access entirely.

Repository identities normalize transport spelling: HTTPS, HTTP, Git SSH, a trailing
slash and `.git` resolve to the same host/path key. GitHub owner/repository case is
normalized; GitLab repository paths retain their case. Credentials, query/fragment
suffixes and GitHub branch URLs are rejected. Opaque IDs such as `sites:<id>` retain
their identity; a short name is never guessed to belong to a repository. Persisted
operator-reviewed aliases support older keys. An unknown key with an already
registered display name must reuse the existing key, explicitly adopt its currently
writable `existingScope`, or choose a distinct name for an unrelated project.
Explicit adoption of an already bound Scope adds a key alias instead of another binding.
Enrollment wire replies keep `projectKey` as the exact request echo for existing
MCP identity attestation, and add `canonicalProjectKey` for the persisted binding.
Updated MCP adapters return that canonical key to models; older adapters can keep
using their aliases and reach the same canonical Scope without a process restart.

Retired duplicate project Scopes remain in the Store to preserve original Revisions
and their references. They are hidden from ordinary Scope lists and reject new
content and Scope recreation. Enrollment projects grant their historical Scopes
read-only access alongside contribution to the canonical Scope. Exact old Revision
IDs remain readable, and old project keys resolve to the canonical write destination.

Pass the returned `projectKey` on every project capture, candidate, or activity.
MCP resolves it through the authenticated registration and refreshes the current
connection. An optional explicit `scope` must match; mismatch rejects the write.
This avoids shared mutable “current project” state across Codex tasks. For user
context, pass its explicit authorized `scope` when multiple writes are granted.
Legacy callers may still supply an authorized Scope directly.

If ownership is unclear, check it before registration. If registration is denied,
unavailable, or absent from the host's tool catalog, defer the project write and
continue the task; never fall back to the user Scope. Retry an unknown outcome with
identical arguments. Static/embedded tenants do not support enrollment registration.
A host must refresh MCP tool metadata after installing the new binary; restarting
Runtime alone does not refresh an already loaded MCP catalog.

## Existing Pages

Historical reclassification keeps the Page ID and creates a new current Revision
in the destination Scope. It preserves exact content, observed time, source
references and facets; the previous Revision retains its original Scope and
actor. The new Revision records the old Revision as provenance. A migration
checks the exact current head and destination before writing, and reads the new
head back afterward. It requires a Store backup and a bounded manifest. The
old Revision remains addressable by exact ID; routine search sees the new head
in its project Scope. Derived summaries and mixed-topic Pages require separate
review because their ownership cannot be inferred from one source title or the
client kind.

The local operator command is `pcp scope-transfer <manifest.json> [--confirm]`.
It requires an explicit absolute `PCP_STORE_PATH`; confirmation also requires
`PCP_SCOPE_TRANSFER_BACKUP` to point to an existing backup. Preview the complete
manifest first. Each entry names a Page ID, exact expected current Revision,
source Scope and destination Scope. The command accepts active sealed Pages and separately reviewed revisioned
Topics without validity history. Topic transfers also preserve the extraction
record, exact source membership and current summarizes links; ordinary
revisioned Pages are not accepted. Repeated previews identify already
transferred entries.

## Duplicate-project reconciliation

The offline operator command is `pcp project-reconcile <manifest.json> [--confirm]`.
First rehearse on a SQLite backup and copies of enrollment and context state. The version-1
manifest names absolute `enrollmentPath`, `enrollmentBackup`, `runtimeSocket` and
`scopeTransferManifest` paths, plus explicit `groups` of `canonicalKey`, `memberKeys`
and `targetScope`. Each target must be an existing reviewed member's Scope; bindings
from different clients require separate ownership review. The transfer manifest
uses the existing exact-head Scope-transfer format and cannot create new Scopes.
When context state exists alongside the Store, supply its absolute `contextBackup`.

Confirmation requires the Console-managed Runtime to be stopped, a matching enrollment
backup and `PCP_SCOPE_TRANSFER_BACKUP` identifying an intact backup of this Store.
Transfers preserve content, metadata, Page IDs, historical Revision IDs and provenance.
Only empty duplicate Scopes can retire. Enrollment then atomically publishes one
binding per project, the legacy key aliases, canonical grants and read-only historical
grants. A changed enrollment file rejects publication. An identical retry recognizes
completed transfers and retirements; it never overrides later enrollment changes.
Activity cards move to the canonical Scope with their IDs, content, versions and
expiry unchanged, so existing optimistic receipts still work. Candidate and synthesis
references to a retiring Scope require separate review and reject the migration
before transfers; their durable retry inputs must not be silently rewritten.
Keep backups and the reviewed manifest outside source control, and read back both
current heads and exact historical Revisions before resuming the managed service.

Console refreshes Scope metadata through a lightweight paginated endpoint. Its
visible Console refreshes the name table periodically and immediately for an unknown
Scope, sharing one in-flight request and bounding retries after failures. This
updates existing labels and filters without re-running the full integrity check.
