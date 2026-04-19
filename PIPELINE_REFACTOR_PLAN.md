# Pipeline Refactor — Demand-Driven Indexing

**Status:** Draft, discussion-aligned. Not yet estimated. No code moves until approved.

**Scope:** Replace the 18-stage eager-parse-then-filter indexing pipeline with a 3-stage demand-driven pipeline. The external-dep pipeline inverts from "walk → parse everything → resolve" to "resolve → demand symbols → parse only what answers demand."

**Motivation:**

1. **Concrete failure.** Full reindex of `go-pocketbase` OOMs at 7GB peak / 402MB allocation failure. Root cause: `modernc.org/sqlite` ships 12 platform-variant .go files of ~10MB each; `modernc.org/libc` ships 1367 .go files with 4MB+ per-platform CGo bindings. We eagerly parse every file in every reached sub-package, even though the user's chain reaches ~10 exported symbols out of ~100K. Platform filtering (already committed) is a narrow correctness fix; the architectural fix is demand-driven parsing.

2. **Existing architectural memo.** `feedback_reachability_based_externals`: "don't index whole node_modules/site-packages/JDK-src; BFS from project imports, index only symbols the app reaches + types needed for the next chain hop." Today's pipeline is the opposite — it walks all reached modules, parses everything in them, and lets the resolver pick a handful of matches at the end.

3. **Accidental pipeline sequencing.** Stages 2–8 of the current pipeline share no data dependency that justifies their ordering. Manifests are seen during file discovery but not read until step 6. Language audit builds from parsed output instead of file extensions. Demand set is built as a post-parse pass instead of accumulated during parse. Many of these can collapse into concurrent single-pass work.

4. **Duplicated AST walks.** Connectors re-parse files the main extractor already walked. Route enrichment runs as a post-write `UPDATE` when it could be emitted at extract time. Both are round-trips through the DB for data that was in RAM moments before.

---

## Current pipeline (as of `feat/resolution-engine` HEAD)

Stages are numbered 1–18 matching the walkthrough in the design discussion. Terse summary here; see conversation log + `indexer/full.rs` for detail.

1. File discovery — `changeset.rs::FullScan`.
2. Parse user files in parallel — `parse_file_with_demand` per file.
3. Language audit log (derived from step 2 output).
4. C/C++ vendored split.
5. Write user files + symbols to DB (origin='project').
6. Build `ProjectContext` from manifests.
7. Write `package_deps` rows.
8. Build R6 demand set (TS only).
9. `parse_external_sources` — walk every dep root, enumerate every .go/.ts/.py/etc. file, parse in parallel, write with origin='external'. **OOM site.**
10. Build combined `Vec<ParsedFile>` (user + external + vendored).
11. `resolve_and_write` pass 1 — resolve refs, write edges, record `ChainMiss` on bail-outs.
12. `expand_chain_reachability` — for each miss, call `Ecosystem::resolve_symbol`, pull more files, parse, extend index.
13. Re-resolve pass 2 if step 12 pulled new files.
14. FTS content index.
15. Code chunking.
16. Connectors — detect + match.
17. Per-language `post_index` hooks.
18. `ANALYZE` + store indexed commit.

### Problems with this layout

- **External parsing is ordering-blind.** Step 9 parses everything it walks; step 11 does the actual filtering by resolving refs. Wasted parse work = OOM trigger.
- **Chain walking is a patch pass.** Steps 11–13 are "resolve, discover we missed files, patch, re-resolve." Chain walking should be the *driver* of external parsing, not a bail-out pass.
- **Demand set is too late.** Step 8 builds demand after parse, and only for TS. It has the right shape but is wired as a second pass rather than an accumulator.
- **Connectors re-walk files.** Many connectors re-parse AST or re-query DB for data the main extractor already saw.
- **Pipeline sequencing is accidental.** Several adjacent stages don't have the data dependency their ordering implies.

---

## Target pipeline — 3 stages

### Stage 1 — Discover + Parse

One tree walk, one parallel parse, all outputs captured.

**Inputs:** project root.

**Outputs:**

- `file_list: Vec<AbsolutePath>` — per-file metadata with language pre-determined by extension.
- `manifests: Vec<ManifestData>` — every `go.mod` / `package.json` / `Cargo.toml` / etc. seen during the walk, read inline.
- `ProjectContext` — active ecosystems, active languages (including embedded), workspace packages. Built from manifests + file extensions, no dependency on parsed ASTs.
- `Vec<ParsedFile>` — user files with all extractor outputs:
  - `symbols`, `refs`, `routes`, `db_sets` — as today.
  - `embedded_regions` — sub-parsed regions with offset metadata, today already produced; surface it as first-class output.
  - `connection_points` — NEW output type. Language-plugin-owned detection of REST clients, route handlers, DI registrations, IPC calls, event subscriptions, MQ handlers. Each plugin emits connection points its language/frameworks recognize. Duplication across plugins accepted (six plugins can each know HTTP client patterns; no shared module).
- `demand_set: DemandMap<ModulePath, Set<SymbolName>>` — accumulated inline during ref emission. Ecosystem-agnostic (Go, Python, Java, C# all contribute), not TS-only as today.
- `active_languages: Set<String>` — includes embedded languages.

**What disappears from the current layout:**

- Current steps 3 (language audit), 8 (R6 demand set) fold into step 2's output.
- Current step 6 (ProjectContext init) runs from step 1's manifest read, before parse.
- Current step 17's per-language `post_index` hooks that emit single-file data move into extractor output.

**Data dependencies within stage 1:**

- Manifest read depends on file walk — concurrent with file-list enumeration.
- `ProjectContext` init depends on manifests — starts as soon as manifests are in.
- Parse depends on `ProjectContext` only for plugin dispatch decisions (e.g. which embedded regions to detect based on active ecosystems) — minimal, easy to hand it forward.

**File-system side effects at end of stage 1:** none. Everything stays in memory for stage 2.

---

### Stage 2 — Link

Demand-driven external parsing + resolution, iterated to fixpoint.

**Inputs:** everything from stage 1 + database handle.

**Core algorithm:**

```
persist user files + symbols to DB (origin='project')
write package_deps from ProjectContext

initialize external_index: per-ecosystem cheap symbol→file index
  (for each reached dep root, scan its files for top-level decl names
  without tree-sitter; see "External index construction" below)

demand = stage_1.demand_set.clone()
parsed_external: HashMap<FileId, ParsedFile>
parsed_set: HashSet<PathBuf>

loop:
  # Phase A: translate demand → file pulls
  files_to_parse = []
  for (module, name) in demand:
    if file = external_index.locate(module, name):
      if file not in parsed_set:
        files_to_parse.push(file)
        parsed_set.insert(file)

  if files_to_parse.is_empty() and resolver_converged:
    break

  # Phase B: parse new files with full tree-sitter extraction
  new_parsed = files_to_parse.par_iter().map(parse_file)
  parsed_external.extend(new_parsed)

  # Phase C: resolve one more iteration of refs
  # Chain walker steps what it can; records new demanded symbols for what it can't
  resolver_result = resolve_iteration(
    user_parsed,
    parsed_external,
    demand,
  )

  demand.extend(resolver_result.new_demanded_symbols)
  resolver_converged = resolver_result.converged

write edges to DB from resolver_result.edges
write external symbols (origin='external')
write external_refs / unresolved_refs
```

**Key properties:**

- Chain walker is inside the loop, not a post-pass. When it can't step past a type, it records the demanded symbol; Phase A of the next iteration pulls its file.
- Fixpoint termination: demand grows monotonically, bounded by external symbol count.
- Internal-only chains resolve on iteration 1 (no demand ever emerges); no extra iterations.
- External chains iterate as many rounds as the chain depth.

**External index construction:**

Two candidates, decision deferred:

- **Regex scan.** For each reached dep root, walk `.go`/`.ts`/`.py`/etc. files, emit `^func X` / `^type X` / `^(var|const) X` / `^class X` / `def X(` matches as `(module, name) → file`. Cheap, fast, memory-small. Risk: misses multi-line decls, build-tag-gated decls, unusual formatting.
- **Header-only tree-sitter parse.** Parse each external file to first-level children of `source_file` only, skip stepping into function/method bodies. Accurate, slower than regex, still far cheaper than full extraction because bodies dominate AST size.

Trade-off is accuracy vs. one-time cost. Pick at implementation time; the call site is one function either way.

**What disappears from the current layout:**

- Current steps 9 (eager `parse_external_sources`), 10 (combined slice build), 11 (pass 1 resolve), 12 (`expand_chain_reachability`), 13 (pass 2 resolve) → all collapse into the stage 2 loop.
- `ChainMiss` as a cross-stage record → gone. Misses are in-loop demand.
- Two-pass resolve dance → one resolver, iterated.

---

### Stage 3 — Connect + Enrich

In-memory connection-point matching, DB-side derived indexes, tidy-up.

**Inputs:**

- Everything from stages 1 and 2.
- Resolved `symbol_id_map` (for connection-point matching that references resolved symbol IDs).

**Outputs:**

- `flow_edges` — in-memory fold of connection points from stage 1. Grouped by `(kind, key)` — `(REST, method+path)`, `(Event, event_name)`, `(IPC, command_name)`, etc. Starts × stops cross-product for each group becomes a `flow_edge` row.
- `fts_files` — trigram content index from file contents captured during stage 1's parse.
- `code_chunks` — windowed splits of file contents, same.
- `concepts` / `db_mappings` — whatever per-language `post_index` hooks still make sense after folding single-file emissions into extractor output.
- `ANALYZE` + `indexed_commit` meta.

**What stays in stage 3 vs. what moves forward:**

- Cross-file / cross-service connection matching stays here — the reduce step needs all connection points from all files.
- Single-file enrichment (route `resolved_route`, SQL `db_mappings`, concept discovery) moves into stage 1 extractor output where the per-language plugin already has the AST in hand.

**What disappears from the current layout:**

- Per-connector tree-sitter re-parse → gone. Connection points come from stage 1 via language plugins.
- DB round-trip for matching → gone. Matcher operates on in-memory collections.
- Route-enrichment `UPDATE` SQL → gone. Emit `resolved_route` at extract time.

---

## Component-level changes

### `ParsedFile` shape

Add:

- `connection_points: Vec<ConnectionPoint>` — `{kind, key, role: Start|Stop, file_range, symbol_qname}`.
- `demand_contributions: Vec<(ModulePath, SymbolName)>` — the subset of refs the accumulator sent to the shared demand set. Kept on the struct for diagnostics.

Existing fields stay as-is.

### `LanguagePlugin` trait

Extend `extract` / `extract_with_demand` to emit `connection_points`. New trait methods:

- `extract_connection_points(&self, tree, source) -> Vec<ConnectionPoint>` — per-language detection. Default impl returns empty.

`post_index` hook stays for cross-file enrichment that genuinely can't move to extract time (e.g. concept discovery that needs the full symbol set).

### `Ecosystem` trait

Existing methods stay:

- `locate_roots`, `walk_root`, `resolve_import`, `resolve_symbol`, `parse_metadata_only`.

New method:

- `build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex` — header-only tree-sitter scan that answers `(module, name) → file_path`. Parses each external file to first-level children of `source_file` only, skipping stepping into function/method bodies. Returns a query-only handle; implementations may cache.

Default impl: falls back to today's `walk_root` + full parse (so un-migrated ecosystems keep working during migration).

### Resolver + chain walker

`resolve/engine.rs::resolve_and_write` splits into:

- `resolve_iteration(...) -> IterationResult` — one pass over user refs, returns `{edges, new_demanded_symbols, converged: bool}`.
- `finalize_and_write(db, edges, unresolved)` — DB write after the loop terminates.

Chain walker gets a new hook: `on_missing_target(module, name) -> RecordDemand`. Today it returns `ChainMiss`; after refactor it returns `RecordDemand`, which the driver feeds back into the demand set.

**Fetch timing: deferred iteration.** The chain walker never synchronously pulls a missing file mid-walk. It records the demand and the next loop iteration's Phase A handles the pull. Rationale: keeps resolver parallelism simple (no mid-walk I/O), keeps the loop the single point that mutates the parsed-external map, costs at most one extra iteration per chain depth — negligible given chains are usually 2–4 hops.

### Connector registry

`connectors/registry.rs::run` changes from "query DB for start/stop data, match, write flow_edges" to "accept pre-collected start/stop lists from `ParsedFile::connection_points`, match, return flow_edges." DB writes happen once in stage 3.

Individual connector modules get much smaller — most of their detection logic moves into language plugins; what's left is the matching logic per connection kind.

### `indexer/full.rs`

Becomes the orchestrator for the three stages. Current 1800-line file shrinks to a pipeline driver — most logic moves into per-stage modules (`stage_discover.rs`, `stage_link.rs`, `stage_connect.rs`).

---

## Rollout approach

No phase numbers, no session estimates yet (wait for architect sizing). Rollout shape:

**Preparatory** — changes that don't affect the pipeline shape but unblock later work. **[DONE]**

- ✅ `ConnectionPoint` / `ConnectionKind` / `ConnectionRole` types in `types.rs`.
- ✅ `connection_points` + `demand_contributions` fields on `ParsedFile` + `ExtractionResult` with empty defaults. 55 construction sites patched.
- ✅ `SymbolLocationIndex` type in `ecosystem/symbol_index.rs` with `insert` / `locate` / `find_by_name` / `extend` / `len` / `is_empty` (7 unit tests).
- ✅ `Ecosystem::build_symbol_index` trait method with default returning empty.
- ✅ `Ecosystem::uses_demand_driven_parse` opt-in flag (default false).

**Core inversion** — the actual pipeline shape change. **[DONE for Go; other ecosystems stay on legacy path]**

- ✅ `GoModEcosystem::build_symbol_index` implemented via header-only tree-sitter scan. Methods keyed `Receiver.Name`, top-level decls captured, function bodies never descended. 10 unit tests.
- ✅ `resolve_and_write` split into `resolve_iteration` + `finalize_resolution`. `ResolutionStats::converged()` predicate added. Old API preserved via wrappers.
- ✅ `parse_external_sources` returns `ExternalParsingResult { parsed, symbol_index, demand_driven_roots, demand_driven_ecosystems }`. Eager walk skipped for ecosystems whose `uses_demand_driven_parse()` returns `true`.
- ✅ `expand_chain_reachability_with_index` — symbol-index-first fanout with legacy `resolve_symbol` fallback for un-migrated ecosystems. Demand-driven ecosystems *never* fall back (would re-trigger OOM).
- ✅ Stage 2 resolve loop in `indexer/full.rs`: `resolve_iteration` → `expand_with_index` → re-resolve → ... up to 8 iterations, exits on `converged()` or zero new files. `finalize_resolution` runs once at the end.
- ✅ go-pocketbase validation: 702 files / 32,122 symbols / 48,597 edges / 45.8s wall-clock. Previously OOMed at 7 GB / 402 MB allocation failure.

**All items closed.**

- ✅ Stage 1 / Stage 3 split: Stage 1 discovery lives in `indexer/stage_discover.rs`, Stage 2 link in `indexer/stage_link.rs`, Stage 3 connect stays inline in `full.rs` (connector registry + FTS/chunks). `full.rs` shrank from 2234 → 1139 lines and is now mostly pipeline orchestration.
- ✅ Ecosystem migration: all source-based package ecosystems are demand-driven (go_mod, npm, pypi, maven, cargo, hex, spm, pub, rubygems, cran, composer, cabal, nimble, cpan, opam, luarocks, zig_pkg, godot_api metadata-only). All source-based stdlib ecosystems also demand-driven: `go-stdlib`, `rust-stdlib`, `cpython-stdlib`, `jdk-src`, `dotnet-stdlib` (metadata-only via dotscope), `erlang-otp`, `elixir-stdlib`, `swift-foundation`, `ruby-stdlib`, `kotlin-stdlib`, `groovy-stdlib`, `scala-stdlib`, `ts-lib-dom`, `php-stubs`, `android-sdk`, `clojure-core` (two-source index: maven for Java interop + regex scanner for `.clj` defs). Eager walk retained only for `posix-headers` / `msvc-headers` / `vba-typelibs` — header/metadata ecosystems with no source-symbol surface.
- ✅ Route enrichment at extract time: `resolved_route = route_template` is written inline in `write::write_parsed_files` — the post-parse `UPDATE routes` SQL round-trip is gone.
- ✅ `expand.rs` cleanup: the `resolve_symbol` fallback + its helpers are deleted; module is ~200 lines, focused on the symbol-index-driven expansion path.
- ✅ Connector flattening — infrastructure landed (`LanguagePlugin::extract_connection_points`, `connectors::from_plugins` bridge, `ConnectorRegistry::run_with_plugin_points`) + all source-scan connectors flattened into plugins via `extract_connection_points` + all DB-lookup connectors (gRPC `*Server` inheritance, Spring DI, REST route handlers, Nestjs/Nextjs/Django/FastAPI/Phoenix/Rails/Laravel) flattened into `LanguagePlugin::resolve_connection_points` post-parse hook. Every plugin's `connectors()` now returns `vec![]`; all connector work flows through the two trait methods. See `CONNECTOR_MIGRATION.md`.
- ✅ Cutover cleanup: the `supports_reachability && !uses_demand_driven_parse` middle tier in `stage_link.rs` is gone — now just demand-driven path + eager walk for the header/metadata holdouts.
- ✅ `indexer/incremental.rs` 3-stage alignment: Stage 3 (connector registry + post_index hooks) now runs after the incremental resolve pass so flow edges stay in sync with changed files.

**Migration per ecosystem.** Ordered by Stack Overflow 2025 developer-survey language popularity, with the OOM case first. Each ecosystem gets a `build_symbol_index` implementation and flips from eager-walk to demand-driven individually, verified against `quality-baseline.json` before the next one starts.

1. ✅ **go_mod** — done. Unblocks the pocketbase OOM case.<br>_Note: `go-stdlib` still on legacy eager walk; migration deferred until a concrete regression justifies touching it (Go stdlib isn't huge by itself)._
2. **npm** — JavaScript + TypeScript (and by extension Vue / Svelte / Angular). Highest-volume ecosystem in the benchmark suite, biggest overall payoff.
3. **pypi** — Python. Second-largest footprint. Pulls in `cpython-stdlib`.
4. **maven** — Java + Kotlin (+ Scala / Clojure / Groovy). JVM stack. Pulls in `jdk-src`.
5. **nuget** — C#. Pulls in `dotnet-stdlib`.
6. **composer** — PHP.
7. **cargo** — Rust. Pulls in `rust-stdlib`.
8. **luarocks** — Lua.
9. **rubygems** — Ruby. Pulls in `ruby-stdlib`.
10. **pub_pkg** — Dart.
11. **spm** — Swift. Pulls in `swift_foundation`.
12. **cran** — R.
13. **hex** — Elixir / Erlang / Gleam. Pulls in `elixir-stdlib`, `erlang-otp`.
14. **cpan** — Perl.
15. **opam** — OCaml.
16. **nimble** — Nim.
17. **zig_pkg** — Zig.
18. **cabal** — Haskell.
19. **clojure_core** — Clojure stdlib (Clojure packages ride on Maven).

Ecosystems with no external-dep surface (SQL, Bash, HTML/CSS, PowerShell, CMake, Dockerfile, Bicep, etc.) are not in scope — they have no `Ecosystem::build_symbol_index` need. They continue to contribute connection points and demand via their language plugins.

**Connector flattening.** Runs *per language plugin* alongside that plugin's ecosystem migration, not as a separate phase across all plugins at once. When a language is being touched for its ecosystem's `build_symbol_index`, the same pass moves its connector detection into `extract_connection_points`. This keeps the work scoped to one plugin at a time and avoids a cross-cutting rewrite of every connector module simultaneously.

- Route-enrichment UPDATE SQL → emit `resolved_route` at extract time. One-shot change, not per-plugin.

**Cutover.**

- Delete old eager path once every ecosystem has a working `build_symbol_index`.
- Delete `expand.rs` once no ecosystem falls back to the chain-miss patch pass.

---

## Decisions (resolved from discussion)

1. **Symbol index construction:** **Header-only tree-sitter parse.** Accurate over raw-speed regex — the one-time cost per external file is small compared to the full-extraction cost we avoid, and the accuracy guarantees (no missed multi-line signatures, no build-tag-gated decl confusion) are worth it.
2. **On-demand fetch during resolver:** **Deferred iteration.** Chain walker records the demand, next loop iteration pulls it. Keeps resolver parallelism simple and concentrates file-pull I/O in the loop driver.
3. **Per-ecosystem migration order:** **Stack Overflow 2025 top-languages list**, with go_mod first as the OOM case. Full order codified in the "Migration per ecosystem" section above.
4. **Plugin authoring cost:** **Move per-plugin, not all at once.** When a language's ecosystem gets migrated to demand-driven external parsing, its connector detection moves into `extract_connection_points` in the same pass. No cross-cutting connector rewrite across all plugins simultaneously.

---

## Non-goals

- Changing the SQLite schema (`symbols`, `refs`, `edges`, `flow_edges`, `routes`, `unresolved_refs`, `external_refs` tables) — stays as-is. This refactor is pipeline shape, not storage shape.
- Changing the MCP / CLI surface — stays as-is.
- Changing the embedding / semantic search path — separate pipeline, not touched here.
- Changing incremental reindex behavior — the three-stage shape applies equally; `indexer/incremental.rs` follows the same cleanup in a later pass.
- Moving the chain walker *out* of resolve — it stays an interior resolver component, just wired differently.

---

## What this refactor does NOT fix (acknowledged scope cuts)

- Parse memory per file — a 10MB CGo file parsed in tree-sitter still allocates a huge AST. Demand-driven parsing avoids parsing those files at all if the user's chain doesn't reach them; if it *does* reach them, we still pay the parse cost for that one file. Parallel parse of multiple huge files in one iteration could still stress memory, but the file count per iteration is bounded by demand size, which is small for typical chains.
- False positives in user demand — if the user writes `sql.Open` but the resolver also considers `fmt.Open` and pulls `fmt.Open`'s file, we waste one parse. Acceptable overhead; real demand is almost always small.
- Cross-language chain walking — today's walker is per-language. This refactor does not change that.
