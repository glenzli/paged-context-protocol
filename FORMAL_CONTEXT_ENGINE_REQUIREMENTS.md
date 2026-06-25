# Formal Context Engine Requirements

## 0. Purpose

Build a PCP-lite context query engine as an optional module inside `markdown-formal`.

The engine is for long-form formal mathematics writing. Given a theorem, conjecture, proof task, or informal mathematical question, it should produce a high-signal context pack for another AI model. The context pack must collect relevant definitions, notation conventions, prior propositions/lemmas/theorems, proof dependencies, source excerpts, and possible logical obstructions, so the reasoning AI does not lose continuity across a 2000-3000 page project.

This is not a full Paged-Context-Protocol runtime. It is a practical query and packaging layer inspired by PCP.

## 1. Protocol References

If the implementation window needs to understand PCP:

- Main PCP spec: `/Users/g4i/lab/paged-context-protocol/PROTOCOL.md`
- English PCP spec: `/Users/g4i/lab/paged-context-protocol/PROTOCOL-en.md`
- PCP README: `/Users/g4i/lab/paged-context-protocol/README.md`

The relevant PCP ideas for this project are:

- Logical Pages: split knowledge into addressable units.
- Consolidated Pages: chapter/volume/book-level summaries or routing containers.
- Logical Address Space: stable IDs and explicit references form an addressable graph.
- Router: model-based logical relevance judgment, not vector similarity.
- Consult: drill down from summary/manifest to source excerpt.
- Shelve/Purge: suppress irrelevant or misleading candidates during a query.
- Context Pack: the synthesized active horizon passed to another AI.

Do not attempt to implement the full protocol state machine in this phase.

## 2. Implementation Location

Implement this in:

`/Users/g4i/lab/markdown-formal`

The reason is that `markdown-formal` already owns the ground truth formal structure:

- `.markdown-formal/dependency-graph.json`
- `.markdown-formal/preview-cache.json`
- `.markdown-formal/definitions.json`
- `.markdown-formal/symbols.json`
- `.markdown-formal/reference-map.md`

The PCP repository should remain the protocol/reference repository. This feature should be a `markdown-formal` optional capability.

## 3. Existing Inputs

The engine should consume existing `markdown-formal` generated files.

### 3.1 Dependency Graph

Path:

`<formal-root>/.markdown-formal/dependency-graph.json`

Current schema includes:

- `nodes[]`
  - `id`
  - `kind`
  - `display`
  - `title`
  - `path`
  - `line`
  - `endLine`
  - `bookKey`
  - `volumeKey`
  - `unitKind`
  - `unitKey`
  - `chapter`
  - `number`
- `edges[]`
  - `from`
  - `to`
  - `kind`
  - `where`: `statement | proof | body`
  - `path`
  - `line`
- `cycles`
- `diagnostics`
- `summary`

Use this as the hard dependency graph. It captures explicit `@h-...` references and must be treated as higher-confidence than model guesses.

### 3.2 Preview Cache

Path:

`<formal-root>/.markdown-formal/preview-cache.json`

Use this for object manifests:

- stable ID
- type
- title
- file path
- line range
- captured statement/preview content
- chapter/book/volume metadata

### 3.3 Definition Table

Path:

`<formal-root>/.markdown-formal/definitions.json`

Use this for explicit definition recall.

The engine should also be able to consume definitions scanned into `preview-cache.json` if available.

### 3.4 Symbol Table

Path:

`<formal-root>/.markdown-formal/symbols.json`

Use this for notation recall. Symbols are high-priority because math failures often come from forgotten notation scope.

### 3.5 Source Markdown

All emitted context excerpts must preserve source paths and line numbers.

Summaries are routing aids only. Source Markdown remains the ground truth.

## 4. Non-Goals

Do not implement these in the first version:

- Full PCP runtime.
- Autonomous theorem proving.
- Automatic rewriting of the source manuscript.
- Global vector database as the primary router.
- Replacing `markdown-formal`'s existing graph tools.
- Storing mathematical truth only in model-generated summaries.

Vector search may be added later as a baseline or auxiliary candidate source, but it must not have routing authority over the model Router or the explicit dependency graph.

## 5. Core Product Shape

Add a new CLI namespace under `npm run formal -- context ...`.

Suggested commands:

```bash
npm run formal -- context build
npm run formal -- context query "..."
npm run formal -- context pack "..." --budget 60000
npm run formal -- context inspect <h-id>
npm run formal -- context explain <pack-id-or-query>
```

### 5.1 `context build`

Build a PCP-lite context index from existing generated files.

Output:

`<formal-root>/.markdown-formal/context-index.json`

It should include:

- page manifests for theorem-like objects
- section/page/chapter/volume/book manifests
- dependency adjacency lists
- reverse dependency adjacency lists
- source location index
- definition index
- symbol index
- optionally generated chapter/volume summary placeholders

This command should run after `prepare`.

### 5.2 `context query`

Run Router analysis and print a concise result:

- interpreted intent
- candidate IDs
- direct dependencies
- upstream dependencies
- downstream impact candidates
- definitions/symbols matched
- recommended source excerpts
- rejected/shelved candidates with reasons

This command is for quick inspection.

### 5.3 `context pack`

Generate a full context pack for another AI.

Output directory:

`<formal-root>/.markdown-formal/context-packs/`

Output file format:

- Markdown first.
- JSON sidecar optional but recommended.

The Markdown pack should be readable and pasteable into another AI window.

### 5.4 `context inspect <h-id>`

Show a full manifest for one formal object:

- display number
- title
- source location
- statement preview
- upstream dependencies
- downstream dependents
- statement/proof/body edge split
- definitions and symbols appearing nearby if available

### 5.5 `context explain`

Show why the Router selected or rejected items for a query.

This is important because PCP-style routing must be auditable.

## 6. Router Philosophy

The Router must be model-based logical relevance judgment, not vector similarity.

This is central.

For formal mathematics, semantic similarity is unreliable because:

- A rejected route and an accepted route may be semantically close.
- A theorem and its obstruction may use the same vocabulary.
- A low-frequency symbol can be logically decisive.
- A statement may depend on an earlier hidden convention rather than a lexically similar theorem.
- Similar-looking objects in different chapters may have incompatible assumptions.
- Vector similarity cannot reliably distinguish statement dependency, proof dependency, and motivational prose.

The engine may use deterministic filters to reduce candidate volume:

- exact symbol matches
- stable IDs
- display references
- chapter/volume/book metadata
- explicit dependency graph
- source path locality
- title/token lexical scan

But final relevance selection should be done by a Router model reading manifests and summaries.

## 7. Model Strategy

Provider must be configurable.

Recommended model roles:

- Cheap Router: DeepSeek Flash, local Qwen, local Gemma, or equivalent.
- Strong Router fallback: DeepSeek Pro or equivalent.
- Consolidator/summarizer: cheap model for drafts, stronger model for high-value summaries.

Do not hardcode model names. Use config fields and environment variables.

Suggested config:

```json
{
  "context": {
    "provider": "deepseek",
    "routerModel": "deepseek-v4-flash",
    "strongModel": "deepseek-v4-pro",
    "apiKeyEnv": "DEEPSEEK_API_KEY",
    "baseUrl": "",
    "maxRouterCandidates": 120,
    "defaultPackBudgetTokens": 60000,
    "enableLocalModel": false,
    "localModelCommand": ""
  }
}
```

If no API key/provider is configured, the engine should still support deterministic graph-only output.

## 8. Query Pipeline

The pipeline should be:

1. Ensure generated files exist.
   - If missing, tell the user to run `npm run formal -- prepare`.
2. Load context index.
   - If absent or stale, run or suggest `context build`.
3. Parse query.
   - Extract probable symbols, IDs, explicit display refs, named concepts, target proof mode.
4. Deterministic candidate expansion.
   - direct ID matches
   - definitions and symbols
   - dependency upstream
   - dependency downstream/impact
   - same chapter/section neighbors
   - chapter/volume overview pages
5. Router model pass.
   - classify each candidate as `hot`, `background`, `consult`, `shelve`, `reject`.
   - require short reasons.
6. Source excerpt loading.
   - load exact source ranges for `hot` and `consult` items.
   - include source path and line number.
7. Pack synthesis.
   - order by logical role, not just file order.
   - include dependency chain and assumptions before proof material.
8. Output Markdown context pack.

## 9. Candidate Classifications

Router output should use a structured schema:

```json
{
  "intent": {
    "summary": "",
    "proofGoal": "",
    "symbols": [],
    "likelyTheoryRegion": []
  },
  "candidates": [
    {
      "id": "h-...",
      "decision": "hot",
      "role": "definition | notation | prerequisite | theorem | lemma | obstruction | downstream-impact | background | rejected",
      "reason": "",
      "confidence": 0.0,
      "needsSourceExcerpt": true
    }
  ],
  "missing": [
    {
      "kind": "definition | assumption | lemma | symbol",
      "description": "",
      "suggestedAction": "consult-source | ask-user | broaden-search"
    }
  ]
}
```

Valid decisions:

- `hot`: include source excerpt.
- `background`: include compact manifest/summary only.
- `consult`: load source excerpt and possibly graph neighbors.
- `shelve`: not needed now, keep out of pack.
- `reject`: likely misleading or logically wrong for this query.

## 10. Context Pack Format

The output Markdown should use this structure:

```markdown
# Formal Context Pack

## Query

<original user query>

## Router Interpretation

<model interpretation, symbols, suspected proof objective>

## High-Priority Definitions

- <term>: <definition excerpt>
  Source: `<path>:<line>`

## Notation and Symbol Conventions

- `<symbol>`: <meaning>
  Source: `<path>:<line>`

## Direct Logical Dependencies

### <display> <title> (`h-...`)

Role: prerequisite / bridge / obstruction / ...
Reason selected: ...
Source: `<path>:<line>`

```markdown
<source excerpt>
```

## Upstream Chain

<ordered dependency chain>

## Possible Obstructions or Caveats

<conditions, exceptions, rejected paths, incompatible assumptions>

## Background Only

<compact list of related but not injected objects>

## Rejected / Shelved Candidates

<short reasons, useful for avoiding repeated wrong recall>

## Source Index

<all included source locations>
```

The pack should be optimized for another AI to reason from it directly.

## 11. Page Model Mapping

Map `markdown-formal` objects to PCP-lite pages:

### OriginalPage

Use for:

- theorem-like object statement
- proof block
- definition
- symbol convention
- anchored remark
- example when explicitly cited

Fields:

- `id`
- `kind`
- `title`
- `display`
- `path`
- `line`
- `endLine`
- `content`
- `bookKey`
- `volumeKey`
- `unitKey`
- `sourceRefs`
- `dependsOn`
- `dependedBy`

### ConsolidatedPage

Use for:

- section
- chapter
- volume
- book
- generated query pack

Fields:

- `id`
- `kind`
- `title`
- `sourceIds`
- `summary`
- `scope`
- `path`
- `bookKey`
- `volumeKey`
- `unitKey`

First version may create ConsolidatedPage manifests without model-generated summaries. Graph and metadata are enough for initial routing.

## 12. Source Excerpt Rules

Mathematics is sensitive to exact wording. Therefore:

- Always keep source path and line number.
- Prefer original excerpt over generated summary for `hot` items.
- Never paraphrase assumptions without also including source.
- Do not truncate quantifiers or conditions.
- If a proof is long, include statement first and proof excerpts only when selected by Router.

The pack may include both:

- statement excerpt
- proof excerpt

But it should label them separately.

## 13. Dependency Semantics

Treat edge placement differently:

- `statement` edge: high-priority dependency, likely needed to understand the theorem's assumptions or formulation.
- `proof` edge: needed for proof reconstruction.
- `body` edge: contextual, lower confidence.

For proof-oriented queries, include both `statement` and `proof` upstream edges.

For definition/statement clarification queries, prefer `statement` edges and symbols/definitions.

## 14. Deterministic Graph Tools Integration

Existing commands:

```bash
npm run formal -- graph summary
npm run formal -- graph focus <h-id> --depth 2
npm run formal -- graph impact <h-id>
npm run formal -- graph upstream <h-id>
npm run formal -- graph bridges
npm run formal -- graph isolated
npm run formal -- graph cycles
npm run formal -- graph matrix chapter
```

The context engine can call shared internal graph functions instead of shelling out.

If reusing internals is too much for v1, read `dependency-graph.json` directly.

## 15. Staleness and Verification

The context engine should check whether generated cache files are stale relative to Markdown source files when feasible.

Minimum behavior:

- If generated files are missing, error with a clear instruction.
- If generated files exist, proceed.
- Optionally warn if source files are newer than cache files.

Recommended command sequence:

```bash
npm run formal -- prepare
npm run formal -- context build
npm run formal -- context pack "..."
```

## 16. Cost Controls

Router cost must be controlled.

Requirements:

- Limit model Router input to manifests, not full source.
- Use deterministic graph expansion before model calls.
- Cap candidate count.
- Use a cheap Router model by default.
- Support graph-only mode with no model call.
- Cache Router results for identical query + index hash.

Suggested cache:

`<formal-root>/.markdown-formal/context-cache.json`

## 17. Privacy and Safety

Because source manuscripts may be unpublished:

- Never call an external model unless provider/API key is explicitly configured.
- Print which provider/model will be used.
- Provide `--offline` or `--graph-only`.
- Do not upload full manuscript by default.
- Router calls should receive only query + candidate manifests unless `--allow-source-router` is explicitly enabled.

## 18. Acceptance Criteria for V1

V1 is complete when:

1. `context build` produces `context-index.json`.
2. `context inspect <h-id>` prints source, dependencies, and dependents.
3. `context query "..."` can produce graph-only candidates without any model.
4. With a configured provider, `context query "..."` returns structured Router decisions.
5. `context pack "..." --budget N` writes a Markdown pack with definitions, symbols, dependencies, excerpts, and source index.
6. Pack output never loses source paths and line numbers.
7. The implementation works on the existing `examples/book*` fixture in `markdown-formal`.
8. Tests cover index loading and at least one pack generation path.

## 19. Suggested Milestones

### Milestone 1: Graph-Only Prototype

- Add `context build`.
- Add `context inspect`.
- Add `context query --graph-only`.
- Add `context pack --graph-only`.

No API calls.

### Milestone 2: Router Provider Interface

- Add provider config.
- Add DeepSeek-compatible OpenAI-style client if appropriate.
- Add local command provider stub.
- Add structured Router JSON parsing.

### Milestone 3: Context Pack Quality

- Add budget-aware pack assembly.
- Add source excerpts.
- Add rejected/shelved candidate explanations.
- Add cache.

### Milestone 4: Formal-Math Real Test

- Run on the real formal-math manuscript.
- Identify missing fields in current generated cache.
- Add only the minimum necessary cache metadata.

## 20. Key Design Constraint

Do not turn this into a generic RAG system.

The value is:

- stable formal IDs
- explicit proof dependency graph
- definitions and symbols
- model-based logical relevance routing
- source-grounded context packs

This should help another AI keep mathematical continuity without requiring the user to repeatedly say "see Chapter X" or "this was defined earlier".

