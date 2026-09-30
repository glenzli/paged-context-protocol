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
