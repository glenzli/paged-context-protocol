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

The writer obtains authorized Scope names on demand. When more than one Scope
permits ingest, MCP capture, candidate, and activity calls require an explicit
`scope`. If ownership or authorization is unclear, the writer checks and defers
the write; it does not use the user Scope as a catch-all. Scope creation and
enrollment grants are separate operator-controlled actions. A new literal Scope
must exist before an enrollment can write to it, and a running session must be
reopened to see newly created read-all Scopes.

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
