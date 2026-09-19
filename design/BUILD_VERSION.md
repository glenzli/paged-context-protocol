# Software versions and build identity

`Cargo.toml` → `[workspace.package].version` is the single software release version.
All PCP crates inherit it, and MCP `initialize.serverInfo.version` uses
`pcp_core::SOFTWARE_VERSION`. The first unified release is **0.3.0**; it replaces
the old Cargo 0.1.0 / manually maintained MCP presentation 0.2.0 split.

PCP `capabilities.protocolVersion`, the negotiated MCP protocol date, and the
Codex/ChatGPT plugin package version are separate contracts. Do not change them
just because the software release changes. Updating local binaries does not
publish a plugin package or prove that a host refreshed its cached tool catalog.

## Runtime diagnostics

- `pcp_describe.buildInfo`: compiled identity of the serving MCP process.
- `pcp_describe.providerBuildInfo`: current backend build read through RPC;
  omitted when an older backend does not report it. A failed read is an error,
  never a cached claim of liveness.
- Console `/api/overview`: `consoleBuildInfo` and `runtime.buildInfo`.
- Console `/api/runtime`: `consoleBuildInfo`, `buildInfo` (Runtime), and
  `reachable`. An unreachable Runtime has no reported build here.
- Console Overview → Endpoint: Runtime and Console version, short commit,
  source state and short source digest.
- `pcp-mcp --version`, `pcp-runtime --version`, `pcp-console --version`: inspect
  that executable without opening the Store, enrollment or a network connection.

Each `BuildInfo` contains `version`, `gitRevision`, `dirty`, `sourceDigest` and
`target`. They are compiled into the executable, so moving HEAD after building
cannot rewrite the identity of an already running process. Dirty/source fields
cover PCP inputs (`Cargo.toml`, `Cargo.lock`, `crates/`, `assets/`, and the included
tool integration document); they do not claim to describe unrelated workspace
edits. The digest includes untracked source files and relative paths, and excludes
`target`, `node_modules`, `.git`, `.DS_Store` and `__pycache__` directories/files.
Runtime data, credentials and plugin caches are outside that input set.

The versioned SHA-256 source digest distinguishes different dirty builds; it is
not an executable checksum or a fingerprint of the compiler, flags, or external
path dependencies. Use artifact SHA-256 when verifying an installed executable.
Without Git, revision and dirty status are unknown (`null`), while the source
digest remains available. A source archive in another repository must not borrow
that repository's commit. Directory and Git-ref dependencies invalidate Cargo's
build identity when files are added/deleted or HEAD/index changes. Symlinked
source inputs fail the build instead of silently hashing a different source tree.

## Release workflow

1. Update the workspace version for a software release and regenerate the local
   package versions in `Cargo.lock`. Use patch bumps for compatible fixes, minor
   bumps for added functionality; keep incompatible pre-1.0 changes in a new
   minor release and describe the migration.
2. Build the three deployed binaries together and validate the affected contracts.
3. Install the MCP executable, restart Console/Runtime and the shared tunnel,
   then compare live diagnostics with the built artifacts. An installed file and
   an existing process may have different identities until restart.
4. Publish changed plugin package metadata through its own release flow when
   applicable, and separately refresh/verify host tool metadata after tool changes.

No version information is added to the recurring model instructions or ordinary
memory receipts. Diagnostic metadata does not participate in capability equality,
so a compatible Runtime rebuild does not itself prevent MCP reconnection.
