# Resolution Engine Architecture Refactor

Introduce `Ecosystem` as a first-class architectural construct. An ecosystem is a package world — one install location, one dep format, one artifact shape — that one or more languages participate in. The current one-locator-per-language shape is wrong: Maven is not Java's, it's five JVM languages'; Hex is not Elixir's, it's three BEAM languages'; npm hosts JS + TS + Vue + Svelte + Angular. Ecosystems absorb the misplaced per-language externals code, the `indexer/manifest/` parsers, and all stdlib indexing. Plugins become pure file analyzers.

## Motivation

Four rules from the architect:

1. `indexer/externals/` must not contain per-language code.
2. External code must be determined dynamically from parsed sources, never from hardcoded name lists.
3. Per-language `externals.rs` files must not exist.
4. Inline matcher lists in resolvers — same rule, lower priority.

Plus the deeper insight from this session:

5. **Ecosystem is the stable unit of package identity**, not language. The current coupling (language → locator, language → manifest parser, language → stdlib list) manufactures accidental duplication (Kotlin locator calls Java locator, Scala is a shim, .m2 scanned three times in a polyglot JVM project).
6. **Plugins analyze files. Ecosystems provide packages. ProjectContext integrates them.** Three concerns, three registries, no circular dependency.

Current state (verified 2026-04-16 against HEAD of `feat/resolution-engine`):

- **23 per-ecosystem files inside `indexer/externals/`** (9,080 lines), named per-language. All 22 source-walking files are already per-ecosystem logic but misindexed by language name. `externals/scala.rs` is a shim that delegates to `JavaExternalsLocator`; `externals/clojure.rs` mostly delegates likewise; Kotlin's plugin calls both `JavaExternalsLocator` *and* its own Android SDK probe.
- **20 parsers in `indexer/manifest/`** parallel to the externals hierarchy. Same topology, different responsibility (parsing vs. walking). Both collapse into ecosystems.
- **43 `languages/*/externals.rs` files** (2,898 lines). 19 are pure hardcoded `const EXTERNALS: &[&str]` lists under 50 lines. The rest are larger hardcoded lists with minor post-processing. Kotlin's is the exception — it houses Android SDK environment probing, which belongs in an ecosystem.
- **~12K lines of per-language `builtins.rs`** across 48 files. Most entries are stdlib identifiers that would resolve through indexed source if stdlib ecosystems existed.
- **One hardcoded test-framework matcher function** in the shared resolver: `test_framework_globals(dep)` at `indexer/resolve/engine.rs:2482` (~109 matcher names), with call sites at 786 and 788. `heuristic.rs` has no hardcoded symbol arrays.
- **Per-language `resolve.rs` matcher density**: cmake 72, python 51, typescript 46, rust 44. Most are syntax keywords, not framework names.
- **Dead artifact**: `LanguagePlugin` trait has a stale comment at `languages/mod.rs:114-115` referring to a removed method. `indexer/framework_globals.rs` is down to `is_test_file()` only. `indexer/primitives.rs` remains a dispatcher that will thin further after this refactor.

## Target architecture

Three orthogonal registries, one project-level integrator:

```
┌─────────────────┐        ┌──────────────────┐
│ LanguageRegistry│        │ EcosystemRegistry │
│  — plugins —    │        │  — ecosystems —   │
│ parse + resolve │        │ discover + walk   │
│ files           │        │ packages          │
└────────┬────────┘        └─────────┬────────┘
         │                           │
         └───────────┬───────────────┘
                     ▼
           ┌─────────────────────┐
           │   ProjectContext    │
           │  per-project seam   │
           │ - active ecosystems │
           │ - language presence │
           │ - manifest scan     │
           │ - file routing      │
           └─────────────────────┘
```

### Directory shape (target)

```
crates/bearwisdom/src/
├── ecosystem/                       # Ecosystem trait + all impls
│   ├── mod.rs                       # trait, EcosystemId, EcosystemKind,
│   │                                # EcosystemActivation, EcosystemRegistry
│   ├── shared/
│   │   ├── tarball.rs               # tar+gzip extraction
│   │   ├── jar.rs                   # extract_java_sources_jar
│   │   ├── cache.rs                 # is_cache_stale, cache paths
│   │   └── walk.rs                  # bounded walker primitives
│   ├── maven.rs                     # Package — Java+Kotlin+Scala+Clojure+Groovy
│   ├── npm.rs                       # Package — JS+TS+Vue+Svelte+Angular
│   ├── pypi.rs                      # Package — Python
│   ├── cargo.rs                     # Package — Rust
│   ├── hex.rs                       # Package — Elixir+Erlang+Gleam
│   ├── nuget.rs                     # Package — C#+F#+VB.NET (metadata-only)
│   ├── pub.rs                       # Package — Dart
│   ├── spm.rs                       # Package — Swift
│   ├── go_mod.rs                    # Package — Go
│   ├── rubygems.rs                  # Package — Ruby
│   ├── composer.rs                  # Package — PHP
│   ├── cran.rs                      # Package — R
│   ├── cabal.rs                     # Package — Haskell
│   ├── opam.rs                      # Package — OCaml
│   ├── luarocks.rs                  # Package — Lua
│   ├── nimble.rs                    # Package — Nim
│   ├── cpan.rs                      # Package — Perl
│   ├── zig_pkg.rs                   # Package — Zig (build.zig.zon)
│   ├── rust_stdlib.rs               # Stdlib — Rust
│   ├── cpython_stdlib.rs            # Stdlib — Python
│   ├── jdk_src.rs                   # Stdlib — Java+Kotlin+Scala+Clojure
│   ├── kotlin_stdlib.rs             # Stdlib — Kotlin
│   ├── android_sdk.rs               # Stdlib — Kotlin+Java (platform SDK)
│   ├── dotnet_stdlib.rs             # Stdlib — C#+F#+VB.NET
│   ├── godot_api.rs                 # Stdlib — GDScript
│   ├── ts_lib_dom.rs                # Stdlib — TypeScript+JavaScript (DOM)
│   ├── ruby_stdlib.rs               # Stdlib — Ruby
│   ├── php_stubs.rs                 # Stdlib — PHP (JetBrains/phpstorm-stubs)
│   └── posix_headers.rs             # Stdlib — C+C++
├── indexer/
│   ├── project_context.rs           # ProjectContext (the seam)
│   ├── resolve/
│   ├── parallel.rs
│   └── ...
└── languages/<lang>/
    ├── mod.rs                       # LanguagePlugin impl
    ├── extract.rs                   # tree-sitter extraction
    ├── resolve.rs                   # language resolver (Tier 1.5)
    ├── chain.rs                     # chain walker (if needed)
    ├── connectors/                  # framework connectors
    └── keywords.rs                  # keywords + operators + intrinsics ONLY
                                     # NO externals.rs
                                     # NO builtins.rs
```

Old `indexer/externals/` and `indexer/manifest/` disappear.

### Trait sketches

The three traits and the seam, in the shape they should land in:

```rust
// ─────────────────────────────────────────────────────────────────
// ecosystem/mod.rs — Ecosystem trait (new)
// ─────────────────────────────────────────────────────────────────

pub trait Ecosystem: Send + Sync {
    /// Stable identifier. Primary key in EcosystemRegistry.
    fn id(&self) -> EcosystemId;

    /// Package (npm, Cargo, Hex) vs Stdlib (rust-stdlib, jdk-src,
    /// android-sdk). Stdlibs are conceptually identical ecosystems,
    /// just with different activation and caching policies.
    fn kind(&self) -> EcosystemKind;

    /// Capability declaration: which language ids appear in this
    /// ecosystem's packages. Used (a) at walk time to route each
    /// file to the right plugin for parsing, and (b) at resolve
    /// time to filter which ecosystems a given ref can reach.
    ///
    /// Not an assignment: a Kotlin file does not intrinsically
    /// "belong to" Maven. Project-level activation and per-file
    /// language detection drive the actual routing.
    fn languages(&self) -> &'static [&'static str];

    /// Manifest formats this ecosystem recognizes. For a Package
    /// ecosystem, these are the lockfiles and dep-declaration
    /// files it parses. For a Stdlib ecosystem, typically empty
    /// (activation is probe-based).
    fn manifest_specs(&self) -> &'static [ManifestSpec];

    /// When is this ecosystem active for a given project?
    /// Evaluated during ProjectContext initialization.
    fn activation(&self) -> EcosystemActivation;

    /// Discover on-disk dep roots. Called once per project during
    /// indexing for every active ecosystem.
    fn locate_roots(&self, ctx: &LocateContext) -> Vec<ExternalDepRoot>;

    /// Walk one dep root. Each WalkedFile is tagged with the
    /// ecosystem id; language detection happens per file by the
    /// indexer (not by the ecosystem).
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile>;

    /// Metadata-only extraction path. Used by NuGet (DLL metadata
    /// via dotscope) and potentially jdk-src (jmod). Mutually
    /// exclusive with walk_root for a given dep.
    fn parse_metadata_only(&self, dep: &ExternalDepRoot)
        -> Option<Vec<ParsedFile>> { None }

    /// Per-file post-processing hook. npm uses this to prefix
    /// symbols with package name. Default: no-op.
    fn post_process_parsed(
        &self, dep: &ExternalDepRoot, parsed: &mut ParsedFile,
    ) {}
}

pub struct EcosystemId(pub &'static str);

pub enum EcosystemKind { Package, Stdlib }

pub struct ManifestSpec {
    pub glob: &'static str,                       // "**/pom.xml"
    pub parse: fn(&Path) -> Result<ManifestData>, // format-specific parser
}

pub enum EcosystemActivation {
    /// Active iff any manifest matching manifest_specs() is found.
    ManifestMatch,
    /// Active iff any file of this language is present in project.
    /// (e.g., rust-stdlib when any .rs file exists.)
    LanguagePresent(&'static str),
    /// Active iff the named manifest has a specific field containing
    /// a value. (e.g., ts-lib-dom when tsconfig.compilerOptions.lib
    /// contains "DOM".)
    ManifestFieldContains {
        manifest_glob: &'static str,
        field_path: &'static str,
        value: &'static str,
    },
    /// Active unconditionally on a given platform. (e.g.,
    /// posix-headers on unix, msvc-headers on Windows.)
    AlwaysOnPlatform(Platform),
    /// Active only when another ecosystem is active. (e.g.,
    /// android-sdk requires Maven active.)
    TransitiveOn(EcosystemId),
    /// Composite: all clauses must match.
    All(&'static [EcosystemActivation]),
    /// Composite: any clause matches.
    Any(&'static [EcosystemActivation]),
}

pub struct LocateContext<'a> {
    pub project_root: &'a Path,
    pub manifests: &'a ManifestScanResult,
    pub active_ecosystems: &'a [EcosystemId],
}

pub struct EcosystemRegistry {
    ecosystems: Vec<Arc<dyn Ecosystem>>,
}
impl EcosystemRegistry {
    pub fn get(&self, id: EcosystemId) -> Option<&Arc<dyn Ecosystem>>;
    pub fn all(&self) -> &[Arc<dyn Ecosystem>];
    /// All ecosystems that declare `lang` in their languages() list.
    pub fn for_language(&self, lang: &str) -> Vec<&Arc<dyn Ecosystem>>;
}
```

```rust
// ─────────────────────────────────────────────────────────────────
// languages/mod.rs — LanguagePlugin trait (updated)
// ─────────────────────────────────────────────────────────────────

pub trait LanguagePlugin: Send + Sync {
    fn id(&self) -> &'static str;

    fn tree_sitter_language(&self) -> tree_sitter::Language;

    /// File recognition: extensions, shebangs, content signatures.
    fn recognizer(&self) -> FileRecognizer;

    /// Extract symbols, refs, scopes from a parse tree.
    fn extract(&self, src: &str, tree: &Tree) -> ExtractedFile;

    /// Embedded regions within a parsed file. Each region declares
    /// an inner language id + byte range. Inherits the host file's
    /// ecosystem set at resolution time — no separate flag needed.
    fn embedded_regions(
        &self, src: &str, tree: &Tree,
    ) -> Vec<EmbeddedRegion> { Vec::new() }

    /// Language-specific resolver (Tier 1.5).
    fn resolver(&self) -> Option<Arc<dyn LanguageResolver>> { None }

    /// Framework connectors owned by this plugin.
    fn connectors(&self) -> Vec<Box<dyn Connector>> { vec![] }

    /// Keywords + operators + compiler intrinsics ONLY.
    /// Never stdlib identifiers. Never framework names.
    fn keywords(&self) -> &'static [&'static str] { &[] }

    /// Coverage metadata.
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }

    /// Post-index enrichment hook.
    fn post_index(
        &self, db: &Database, project_root: &Path, ctx: &ProjectContext,
    ) {}
}

// REMOVED from the trait (replaced by Ecosystem or dead):
//   externals_locator()     → moved to Ecosystem
//   externals()              → moved to Ecosystem (Stdlib kind)
//   primitives()             → merged into keywords()
//   builtin_type_names()     → merged into keywords()
```

```rust
// ─────────────────────────────────────────────────────────────────
// indexer/project_context.rs — ProjectContext (the seam)
// ─────────────────────────────────────────────────────────────────

pub struct ProjectContext {
    pub root: PathBuf,

    /// Ecosystems whose activation() returned true for this project.
    pub active_ecosystems: Vec<EcosystemId>,

    /// Languages detected in the project (from file walk).
    pub language_presence: HashSet<String>,

    /// Parsed manifests, keyed by (ecosystem_id, manifest_path).
    pub manifest_scan: ManifestScanResult,

    /// Cache of dep roots resolved by each active ecosystem.
    pub dep_roots: HashMap<EcosystemId, Vec<ExternalDepRoot>>,
}

impl ProjectContext {
    /// Construct by walking project root, running every ecosystem's
    /// activation predicate, and evaluating locate_roots for each
    /// active ecosystem.
    pub fn initialize(
        root: &Path,
        langs: &LanguageRegistry,
        ecos: &EcosystemRegistry,
    ) -> Result<Self>;

    /// Given a file's language id, which ecosystems can its refs
    /// reach? Filters active_ecosystems by Ecosystem::languages().
    pub fn resolvable_ecosystems(
        &self, file_lang: &str,
    ) -> Vec<EcosystemId>;

    /// Tag a walked file with the ecosystem that produced it.
    /// Walked files always come from a specific ecosystem walk.
    pub fn ecosystem_for_walked_file(
        &self, walk_origin: EcosystemId,
    ) -> EcosystemId { walk_origin }

    /// For a project-local (non-walked) file, no ecosystem tag —
    /// resolution uses resolvable_ecosystems(file_lang) at query time.
    pub fn is_walked_from(&self, path: &Path) -> Option<EcosystemId>;
}
```

### How the pieces interact

**Indexing a project:**

1. `ProjectContext::initialize` walks project root once, builds language_presence + manifest_scan.
2. For each ecosystem in the registry, evaluate `activation()`. Keep those that return true.
3. For each active ecosystem, call `locate_roots()`. Cache the result.
4. Parse every project file: primary plugin by recognizer, extract symbols+refs+embedded_regions, recurse into embedded regions.
5. For each active ecosystem, walk every dep root. Per walked file: detect language, route to plugin, parse, tag with `origin='external'` and `ecosystem_id`.

**Resolving a ref:**

1. Ref has `(language, source_file)`.
2. If source_file is walked (external), ecosystem is already tagged; resolve within that ecosystem's symbol universe + project symbols.
3. If source_file is project-local, compute `resolvable_ecosystems(language)` and search across that union.
4. Tier 1 (qname match in project or reachable ecosystems) → done.
5. Tier 1.5 (language resolver chain walking with type info) → done.
6. Tier 2 (heuristic) — unchanged.

**Embedded regions inherit ecosystem set:**

A `<script lang="ts">` inside a `.vue` file belongs to the same project and resolves against the same npm packages as the Vue file. The inner plugin (TypeScript) invokes resolution through the same ProjectContext — no flag or ecosystem override needed. This falls out of the design because ecosystem filtering is language-driven, not file-driven.

### Polyglot pressure tests (design verification)

| Case | Outer plugin | Embedded | Active ecosystems | Notes |
|---|---|---|---|---|
| `.vue` in monorepo with `package.json` | Vue | TS, CSS | npm, ts-lib-dom | TS region resolves via npm |
| `.razor` in `.csproj` project | Razor | C# | NuGet, dotnet-stdlib | C# resolves against NuGet |
| `.heex` in Phoenix app | Heex | HTML, Elixir | Hex | Recursive embed OK |
| `.ipynb` with Python kernel | Jupyter | Python cells, Markdown | PyPI, cpython-stdlib | Cells share deps |
| `.mdx` in Next.js site | MDX | Markdown, JSX | npm, ts-lib-dom | — |
| `.java` + `.kt` in Gradle project | each plugin | — | Maven, jdk-src, android-sdk, kotlin-stdlib | Shared ecosystem set |
| `.py` with SQL string | Python | SQL literal | PyPI | SQL has no ecosystem |
| `.ex` with embedded Heex sigil | Elixir | Heex | Hex | Recursive discovery |
| `.ts` walked from `node_modules/@scala/foo/` | TypeScript | — | npm (tagged) | walk_origin authoritative |
| Monorepo: pyproject.toml + package.json + go.mod | each per file | — | PyPI, npm, go mod | Three parallel ecosystem sets; per-file filter picks the right one |
| Kotlin Multiplatform with Android target | Kotlin | — | Maven, android-sdk, kotlin-stdlib | kotlin-stdlib stays active regardless of Android |
| Rust workspace with no stdlib installed | Rust | — | Cargo, rust-stdlib (attempted) | rust-stdlib activation fails gracefully; unresolved rises honestly |

No case forces a cycle. The model holds.

## Phases

Ten phases. Each is a separate commit or small commit range with a measurable gate. The critical path is 1→2→4→5; 6 and 7 parallelize per-language.

### Phase 0 — Prep and baseline (0.5 session, solo)

**Goal**: lock a regression baseline and clean dead artifacts.

1. `resolution-baseline.json` already exists. Copy to `resolution-baseline-pre-refactor.json`. Commit, tag `pre-ecosystem-refactor`.
2. Save per-language unresolved top-30s as regression canaries.
3. Delete the stale deprecated comment at `languages/mod.rs:114-115` (five-minute freebie).
4. Audit this plan's Inventory appendix (section 10) against current code — re-run the `find`/`grep` commands, update any drift.

**Output**: tag + baseline snapshot + clean mod.rs comment.

### Phase 1 — Define the new traits (1 session, solo)

**Goal**: introduce `Ecosystem`, `EcosystemRegistry`, `ProjectContext`, updated `LanguagePlugin`. No behavior change. Existing `ExternalSourceLocator` impls continue to work behind a shim.

1. Create `crates/bearwisdom/src/ecosystem/mod.rs` with the trait definitions from the sketch above (Ecosystem, EcosystemId, EcosystemKind, EcosystemActivation, ManifestSpec, LocateContext, EcosystemRegistry).
2. Move shared helpers from `indexer/externals/mod.rs` into `ecosystem/shared/{tarball,jar,cache,walk}.rs`. Re-export from `ecosystem/mod.rs`.
3. Create `indexer/project_context.rs` with the `ProjectContext` sketch. Start with just `initialize()` that does a manifest scan + language presence scan + activation evaluation. Discovery (dep root caching) in Phase 2.
4. For every current `ExternalSourceLocator` impl, wrap it in a `LegacyLocatorEcosystem` shim that implements `Ecosystem` by delegating to the old trait. Register each shim with the EcosystemRegistry, keyed by language id (not final ecosystem id — that's Phase 2).
5. Keep `LanguagePlugin::externals_locator()` intact for now. Indexer reads *both* paths during the transition: iterate EcosystemRegistry OR call the old locator, whichever fires first. This lets us migrate ecosystem-by-ecosystem.

**Success gate**: `cargo test --workspace` passes. Fleet reindex: unresolved counts match Phase 0 baseline within ±10 refs per project.

**Output**: one commit, additive only.

### Phase 2 — Ecosystem consolidation (2 sessions, solo or parallel agents per ecosystem)

**Goal**: replace the 1-to-1 language→locator shims with real Ecosystem impls that span multiple languages.

For each target ecosystem, implement a real `Ecosystem` trait impl with proper `languages()`, `manifest_specs()`, `activation()`, `locate_roots()`, `walk_root()`. Delete the shims as they're replaced. Merge files as listed:

| Ecosystem | Merges (current files) | Languages |
|---|---|---|
| **MavenEcosystem** | `externals/java.rs` + `externals/clojure.rs` + `externals/scala.rs` + Kotlin's JavaLocator delegation + Groovy | java, kotlin, scala, clojure, groovy |
| **NpmEcosystem** | `externals/typescript.rs` (+ js/vue/svelte/angular already routed via it) | typescript, tsx, javascript, vue, svelte |
| **HexEcosystem** | `externals/elixir.rs` + `externals/erlang.rs` + `externals/gleam.rs` | elixir, erlang, gleam |
| **CargoEcosystem** | `externals/rust_lang.rs` | rust |
| **PypiEcosystem** | `externals/python.rs` | python |
| **NugetEcosystem** | `externals/dotnet.rs` (metadata-only via dotscope) | csharp, fsharp, vbnet |
| **PubEcosystem** | `externals/dart.rs` | dart |
| **SpmEcosystem** | `externals/swift.rs` | swift |
| **GoModEcosystem** | `externals/go.rs` | go |
| **RubygemsEcosystem** | `externals/ruby.rs` | ruby |
| **ComposerEcosystem** | `externals/php.rs` | php |
| **CranEcosystem** | `externals/r_lang.rs` | r |
| **CabalEcosystem** | `externals/haskell.rs` | haskell |
| **OpamEcosystem** | `externals/ocaml.rs` | ocaml |
| **LuarocksEcosystem** | `externals/lua.rs` | lua |
| **NimbleEcosystem** | `externals/nim.rs` | nim |
| **CpanEcosystem** | `externals/perl.rs` | perl |
| **ZigPkgEcosystem** | `externals/zig.rs` | zig |

Ordering: do MavenEcosystem and NpmEcosystem first — they consolidate the most duplication and exercise the multi-language case most heavily.

For each merge, delete the per-language `externals_locator()` override. The Kotlin Android SDK probe stays in code temporarily — it becomes its own ecosystem in Phase 5.

**Success gate**: After each ecosystem lands, fleet reindex; unresolved must match Phase 1 within ±50 refs per project. After all ecosystems land, `indexer/externals/` is empty except for `mod.rs` re-exports (which get fully deleted in Phase 3).

**Output**: 18 commits (one per ecosystem) or bundled by ecosystem-family.

### Phase 3 — Fold `indexer/manifest/` into ecosystems (1 session, solo)

**Goal**: each ecosystem owns its manifest parsers. No more parallel hierarchy.

Every parser in `indexer/manifest/` moves into its ecosystem's `manifest_specs()`:

| Parser | Destination |
|---|---|
| `manifest/cargo.rs` (Cargo.toml, Cargo.lock) | CargoEcosystem |
| `manifest/npm.rs` (package.json, lockfiles) | NpmEcosystem |
| `manifest/maven.rs` (pom.xml) | MavenEcosystem |
| `manifest/gradle.rs` (build.gradle, build.gradle.kts) | MavenEcosystem |
| `manifest/sbt.rs` (build.sbt) | MavenEcosystem |
| `manifest/clojure.rs` (deps.edn, project.clj) | MavenEcosystem |
| `manifest/mix.rs` (mix.exs) | HexEcosystem |
| `manifest/pyproject.rs` | PypiEcosystem |
| `manifest/nuget.rs` (.csproj, .fsproj, packages.config) | NugetEcosystem |
| `manifest/composer.rs` | ComposerEcosystem |
| `manifest/gemfile.rs` | RubygemsEcosystem |
| `manifest/pubspec.rs` | PubEcosystem |
| `manifest/swift_pm.rs` | SpmEcosystem |
| `manifest/go_mod.rs` | GoModEcosystem |
| `manifest/opam.rs` | OpamEcosystem |
| `manifest/rockspec.rs` | LuarocksEcosystem |
| `manifest/zig_zon.rs` | ZigPkgEcosystem |
| `manifest/description.rs` | CranEcosystem |
| `manifest/gleam.rs` | HexEcosystem |

Delete `indexer/manifest/` entirely. Each ecosystem declares its manifest parsers via `manifest_specs() -> &[ManifestSpec]`.

One subtle case: `build.gradle.kts` is a Kotlin-syntax Gradle script. MavenEcosystem uses KotlinPlugin's parser as a *tool* to tokenize the build script, then extracts deps from the AST. Parser is borrowed from the plugin; the parsing *logic* (what to extract) lives in the ecosystem. This is the one place plugin↔ecosystem coupling exists, and it's bounded: ecosystems can call `LanguageRegistry.get(lang).parse()` to reuse tree-sitter grammars without taking any other plugin responsibilities.

**Success gate**: `cargo test --workspace` passes. Fleet reindex within ±10 refs per project vs Phase 2.

**Output**: 1–2 commits.

### Phase 4 — Wire ProjectContext as the seam (1 session, solo)

**Goal**: make ProjectContext the single path by which the indexer and resolver reach ecosystems. Remove the Phase-1 transition dual-path.

1. Update the full indexer (`indexer/full.rs`) to construct `ProjectContext::initialize()` once per project, then iterate `ctx.active_ecosystems` for externals indexing.
2. Update the incremental indexer (`indexer/incremental.rs`) to refresh `ProjectContext` on manifest changes.
3. Update the resolver (`indexer/resolve/engine.rs`) to call `ctx.resolvable_ecosystems(lang)` when looking up refs.
4. Embedded-region recursion inherits the same ProjectContext; no per-region ecosystem override.
5. Delete `LanguagePlugin::externals_locator()` from the trait. Every call site now goes through EcosystemRegistry.
6. Walked-file tagging: every file walked via `Ecosystem::walk_root()` gets `(origin='external', ecosystem=<id>, language=<detected>)`. The `ecosystems` column joins to `symbols` for query-side filtering.

**Success gate**: fleet reindex matches Phase 3 within ±10 refs. Delete Phase-1 dual-path code. Code search confirms `externals_locator` is gone from the trait.

**Output**: 1–2 commits.

### Phase 5 — Stdlib ecosystems (3–4 sessions, one per stdlib; parallelizable per ecosystem)

**Goal**: every stdlib becomes an Ecosystem with `kind() = Stdlib`. Probe-based activation. Indexes real compiler/runtime source.

Order by impact × tractability:

| Order | Ecosystem | Probe | Source | Languages | Consumes builtins lines |
|---|---|---|---|---|---|
| 1 | **GodotApi** | Godot install path (env var or registry) | `extension_api.json` | gdscript | ~400 of 548 |
| 2 | **KotlinStdlib + AndroidSdk** | `$KOTLIN_HOME` + `$ANDROID_HOME` | `kotlin-stdlib-sources.jar` + `android.jar` | kotlin, java | ~300 of 434 (kotlin), feeds into future java trims |
| 3 | **RustStdlib** | `rustc --print=sysroot` | `lib/rustlib/src/rust/library/{std,core,alloc}/**/*.rs` | rust | ~400 of 516 |
| 4 | **CpythonStdlib** | `python -c 'import sys; print(sys.prefix)'` | `lib/python*/` .py stdlib | python | ~200 of 276 |
| 5 | **JdkSrc** | `$JAVA_HOME/lib/src.zip` or jmod | `src.zip` | java, kotlin, scala, clojure | ~150 of 224 (java); augments others |
| 6 | **TsLibDom** | TS install location | `lib.dom.d.ts` + `@types/node` | typescript, javascript | ~150 of 228 |
| 7 | **RubyStdlib** | `ruby -e 'print RbConfig::CONFIG["rubylibdir"]'` | stdlib .rb files | ruby | ~300 of 428 |
| 8 | **PosixHeaders / MsvcHeaders** | `/usr/include`, `$VCINSTALLDIR/include` | headers | c, cpp | ~1,100 of 1,528 |
| 9 | **DotnetStdlib** | `dotnet --info` → shared framework | .NET reference assemblies (metadata via dotscope) | csharp, fsharp, vbnet | ~280 of 389 (fsharp), ~60 of 111 (csharp) |
| 10 | **PhpStubs** | bundled JetBrains phpstorm-stubs | stubs source | php | ~180 of 264 |
| 11 | **ScalaStdlib** | maven-resolved `scala-library-sources.jar` | jar | scala | ~180 of 263 |
| 12 | **GroovyStdlib** | `$GROOVY_HOME/lib/groovy-*-sources.jar` | jar | groovy | ~60 of 104 |
| 13 | **ClojureCore** | Maven-resolved `clojure-core-sources.jar` | jar | clojure | ~500 of 631 |
| 14 | **ErlangOtp** | Hex-resolved OTP packages + bundled | erlang stdlib | erlang | ~180 of 248 |
| 15 | **ElixirStdlib** | Hex-resolved `elixir` package | elixir stdlib | elixir | ~450 of 581 |
| 16 | **SwiftFoundation** | SwiftInterface files from SPM checkout / Xcode SDK | swiftinterface | swift | ~140 of 209 |
| 17 | **VbaTypelibs** | OLE typelib introspection (Windows-only) | typelib | vba | ~650 of 858 |

Per-ecosystem task:

1. Implement `Ecosystem` trait: probe in `locate_roots`, walk source in `walk_root`.
2. Register with EcosystemRegistry.
3. Run `bw reindex` on a project in that language. Verify stdlib symbols appear with `origin='external'`, `ecosystem='<stdlib-id>'`.
4. Commit.

No builtins removal yet — that's Phase 7. Phase 5 only *adds* real indexed stdlib; Phase 7 removes the now-redundant hardcoded names.

**Success gate per ecosystem**: project-specific unresolved count drops (or at least doesn't rise) after activation.

**Output**: 10–17 commits (one per ecosystem).

### Phase 6 — Delete per-language `externals.rs` (1 session, solo; partially parallelizable)

**Goal**: every file under `languages/*/externals.rs` is gone. 43 files, 2,898 lines.

**Tier A — bulk-delete (≤50 loc, pure `const` arrays, 19 files):**
`ada`, `bash`, `bicep`, `csharp` (already 3-line stub), `dockerfile`, `fsharp` (stub), `gdscript`, `gleam`, `haskell`, `java` (stub), `matlab`, `ocaml`, `php`, `rust_lang` (stub), `scala` (stub), `scss`, `svelte`, `typescript` (tiny), `vue`. Delete them all in one commit. Any per-language fallout is caught by fleet reindex.

**Tier B — delete after that language's Phase 5 step landed:**
- `javascript/externals.rs` (282) → requires TsLibDom + NpmEcosystem in place for DOM + jQuery equivalents.
- `c_lang/externals.rs` (232) → requires PosixHeaders.
- `clojure/externals.rs` (191) → requires ClojureCore.
- `kotlin/externals.rs` (159) → the Android SDK logic was moved to AndroidSdk ecosystem in Phase 5; hardcoded list can be deleted.
- `erlang/externals.rs` (137) → requires ErlangOtp.
- `groovy/externals.rs` (129) → requires GroovyStdlib.
- `prolog/externals.rs` (120) → per-ecosystem if one is added, else accept hardcoded and move on.
- `robot/externals.rs` (120) → Robot Framework resource files via its own ecosystem, or accept hardcoded.
- `fortran/externals.rs` (111) → intrinsics only; move to `keywords.rs` (see Phase 7 rule).
- Others (dockerfile 92, nim 91, bicep 88, zig 86, hcl 69, nix 68, odin 63, starlark 60, elixir 59, ruby 55, powershell 51, perl 51, lua 49) — audit each; most content will migrate to `keywords.rs` if it's truly language primitives, or be deleted if the respective ecosystem covers it.

**Success gate per deletion**: unresolved on affected projects rises no more than 5%. Rerun fleet reindex after each batch.

**Output**: 1 bulk commit for Tier A + 1 per language for Tier B.

### Phase 7 — Trim `builtins.rs` per language → `keywords.rs` (multi-session, one per language)

**Goal**: every `languages/<lang>/builtins.rs` becomes `keywords.rs` containing keywords + operators + compiler intrinsics ONLY. Stdlib names gone (indexed in Phase 5). 48 files touched.

Same ordering as the Phase 5 stdlib ecosystems — they're paired. After a language's stdlib is indexed, its builtins can be trimmed.

Rule for what stays:
| Keep | Move to indexed source (Phase 5) |
|---|---|
| Language keywords (`if`, `fn`, `def`, `class`) | Stdlib function names |
| Operators (`+`, `->`, `..`) | Framework DSL names |
| Compiler intrinsics (`@This`, `__builtin_*`, `#[derive]`) | Package-API names |
| Primitive type names with no indexable source (`i32`, `bool`) | Test-framework matchers |
| Syntax literals (`true`, `false`, `null`) | |

Target: total of ~48 files sums to ≤5K lines (down from 12K).

**Success gate per language**: unresolved on projects in that language rises no more than 5%.

**Output**: 1 commit per language, ~15 commits.

### Phase 8 — Shared-resolver cleanup (1 session)

**Goal**: delete the remaining hardcoded shared lists.

1. Delete `test_framework_globals` (`engine.rs:2482`), `build_test_globals_union` (`engine.rs:786`), `build_test_globals_by_pkg` (`engine.rs:788`), and the `Some("test_framework")` origin classification (`engine.rs:1445-1450`). Gated on Task 9a landing (forces TS extractor to emit real method chains — prerequisite for matchers resolving via indexed jest/vitest packages in NpmEcosystem).
2. Prune `indexer/primitives.rs`: the `externals()` merge branch (lines 28–31) is dead post-Phase 6; delete it. The `primitives()` merge branch is also dead post-Phase 7; delete that too. The module reduces to a thin wrapper over `keywords()` — or disappears entirely if call sites can inline `plugin.keywords()`.
3. `indexer/framework_globals.rs` — already down to `is_test_file()`. Rename the file to `test_file_detection.rs` for clarity; no logic change.

**Success gate**: `grep -rn "test_framework_globals\|build_test_globals" crates/bearwisdom/src/indexer/` returns zero.

**Output**: 1 commit.

### Phase 9 — Inline matcher cleanup in per-language `resolve.rs` (deferred)

**Goal**: move syntax-keyword inline matcher lists in per-language resolvers into their `keywords.rs`.

Targets: `cmake/resolve.rs` 72 string literals, `python/resolve.rs` 51, `typescript/resolve.rs` 46, `rust_lang/resolve.rs` 44. Most are language keywords (`CACHE`, `PUBLIC`, `PRIVATE` in CMake; stdlib fast-paths in TS/Rust/Python).

This is a move, not a delete: the resolver still matches against them; the list just lives in `keywords.rs` alongside the others. Lowest leverage; do last.

**Output**: 1 commit or skipped.

## Dependency graph

```
Phase 0 (baseline)
   ↓
Phase 1 (traits + shims)
   ↓
Phase 2 (ecosystem consolidation)
   ↓
Phase 3 (fold manifest/)
   ↓
Phase 4 (ProjectContext wiring + drop dual path)
   ↓
Phase 5 (stdlib ecosystems)      ← parallelizable across ecosystems
   ↓
Phase 6 (delete externals.rs)    ← Tier A immediately, Tier B per-language gated on 5
   ↓
Phase 7 (trim builtins.rs)       ← one per language, parallelizable
   ↓
Phase 8 (shared resolver cleanup; gated on Task 9a)
   ↓
Phase 9 (inline matchers — optional)
```

## Risk and rollback

| Risk | Mitigation |
|---|---|
| Stdlib source missing on the machine (rust-src component not installed, `$ANDROID_HOME` unset, etc.) | Ecosystem's activation() fails gracefully; project reports "stdlib unavailable" in diagnostics; unresolved rises honestly |
| Phase 2 merging loses a language-specific nuance | Each merge has its own commit; `git revert` restores the per-language locator during the Phase-1 dual-path window |
| Phase 3 manifest fold breaks dep detection (parser moved, call site missed) | `cargo test --workspace` + fleet reindex per phase |
| ProjectContext initialization becomes a hot path | Cache the manifest scan and activation eval keyed on project_root + last-modified; reuse across incremental-index events |
| Walking stdlib tanks first-index time | Stdlib ecosystems cache aggressively keyed on (ecosystem_id, toolchain_version, source_hash); indexed once per install, not per project |
| Plugin↔ecosystem coupling (Phase 3 Gradle-Kotlin case) creeps beyond build scripts | One bounded escape hatch: `Ecosystem::parse_tool(lang, bytes)` that only returns a tree. No resolution, no symbols. Reviewed at trait-change time |
| User's machine misses multiple language installs → many environment gaps look like regressions | Diagnostics report separates "ecosystem inactive (not installed)" from "ecosystem active but unresolved"; accept that BearWisdom's resolution depends on toolchains being present — this is correct behavior |

Rollback strategy: each phase is its own commit range. `git revert <range>` per phase. The Phase-1 dual-path design specifically supports per-ecosystem rollback during Phase 2.

## Success criteria

- `find crates/bearwisdom/src/languages -name 'externals.rs'` → zero files (down from 43).
- `find crates/bearwisdom/src/languages -name 'builtins.rs'` → zero files; replaced by `keywords.rs` (down from 48 files, 12,021 lines → ≤5,000 lines total).
- `crates/bearwisdom/src/indexer/externals/` and `crates/bearwisdom/src/indexer/manifest/` do not exist.
- `crates/bearwisdom/src/ecosystem/` contains only ecosystem-named files (maven.rs, npm.rs, pypi.rs, cargo.rs, hex.rs, nuget.rs, pub.rs, spm.rs, go_mod.rs, rubygems.rs, composer.rs, cran.rs, cabal.rs, opam.rs, luarocks.rs, nimble.rs, cpan.rs, zig_pkg.rs) plus stdlib ecosystems (rust_stdlib.rs, cpython_stdlib.rs, jdk_src.rs, kotlin_stdlib.rs, android_sdk.rs, dotnet_stdlib.rs, godot_api.rs, ts_lib_dom.rs, ruby_stdlib.rs, php_stubs.rs, posix_headers.rs, plus the specialized long-tail). No per-language files.
- `grep -rn "externals_locator\|test_framework_globals\|build_test_globals" crates/bearwisdom/src/` → zero matches.
- `LanguagePlugin` trait surface is: `id`, `tree_sitter_language`, `recognizer`, `extract`, `embedded_regions`, `resolver`, `connectors`, `keywords`, `symbol_node_kinds`, `ref_node_kinds`, `post_index`. No externals, no primitives, no builtin_type_names.
- `resolution-baseline.json` fleet-wide unresolved within ±5% of pre-refactor on the same machine (controlling for stdlib presence).
- All tests pass (`cargo test --workspace`).

## Sizing

Pessimistic, in "sessions" (one session = one focused architect pass + agent execution):

| Phase | Sessions | Notes |
|---|--:|---|
| 0 | 0.5 | baseline + cleanup |
| 1 | 1 | trait scaffolding |
| 2 | 2 | ecosystem consolidation; biggest cognitive load |
| 3 | 1 | mechanical rename/move |
| 4 | 1 | ProjectContext wiring |
| 5 | 3–4 | stdlib ecosystems; parallelizable with agent fan-out |
| 6 | 1 | mostly bulk-delete |
| 7 | 2–3 | per-language builtins trims |
| 8 | 1 | cleanup |
| 9 | 0.5 | optional |

Total: **12–15 sessions wall-clock**, **5–7 sessions with aggressive agent parallelization** in Phases 5 and 7.

## What's not in scope

- Resolver architecture changes (Tier 1 / 1.5 / heuristic). This refactor changes where symbols *come from*, not how they're matched.
- New language plugins. Only existing plugins migrate.
- Performance work. Expected neutral-to-positive (fewer duplicate scans); measure after Phase 4.
- Cross-ecosystem packages (e.g., a single package that publishes to both npm and PyPI with shared content). Handle per-ecosystem; no special cross-ecosystem primitive yet.
- Connector refactor. Connectors stay at plugin level per prior decision.

## Open design points deferred

- **Stdlib vs Package trait split**: kept as one trait with `kind()` marker. If the two kinds grow substantially different behaviors (e.g., stdlibs need stricter cache invalidation), split in a future refactor.
- **Multi-platform SDK sharing**: Android SDK and Kotlin stdlib are separate ecosystems today. If more platform SDKs emerge (embedded, Apple, WebAssembly runtime), consider a `PlatformSdk` super-trait. Not now.
- **Ecosystem versioning**: if a project pins a specific stdlib version different from what's installed, currently we index what's installed. Honoring pinned versions means snapshotting source per project, which is out of scope.

---

## Inventory appendix

*Populated 2026-04-16 against HEAD of `feat/resolution-engine`. Re-run the audit commands before starting Phase 1 to confirm no drift.*

### A1. `indexer/externals/` — 23 files, 9,080 lines (to disappear)

| File | LOC | Locator struct | Target ecosystem |
|---|--:|---|---|
| `mod.rs` | 433 | — (shared) | `ecosystem/mod.rs` + `ecosystem/shared/*` |
| `clojure.rs` | 164 | `ClojureExternalsLocator` | MavenEcosystem |
| `dart.rs` | 815 | `DartExternalsLocator` | PubEcosystem |
| `dotnet.rs` | 877 | `DotNetExternalsLocator` | NugetEcosystem (metadata-only) |
| `elixir.rs` | 360 | `ElixirExternalsLocator` | HexEcosystem |
| `erlang.rs` | 739 | `ErlangExternalsLocator` | HexEcosystem |
| `gleam.rs` | 93 | `GleamExternalsLocator` | HexEcosystem |
| `go.rs` | 388 | `GoExternalsLocator` | GoModEcosystem |
| `haskell.rs` | 238 | `HaskellExternalsLocator` | CabalEcosystem |
| `java.rs` | 198 | `JavaExternalsLocator` | MavenEcosystem |
| `lua.rs` | 124 | `LuaExternalsLocator` | LuarocksEcosystem |
| `nim.rs` | 181 | `NimExternalsLocator` | NimbleEcosystem |
| `ocaml.rs` | 118 | `OcamlExternalsLocator` | OpamEcosystem |
| `perl.rs` | 173 | `PerlExternalsLocator` | CpanEcosystem |
| `php.rs` | 159 | `PhpExternalsLocator` | ComposerEcosystem |
| `python.rs` | 609 | `PythonExternalsLocator` | PypiEcosystem |
| `r_lang.rs` | 501 | `RLangExternalsLocator` | CranEcosystem |
| `ruby.rs` | 710 | `RubyExternalsLocator` | RubygemsEcosystem |
| `rust_lang.rs` | 560 | `RustLangExternalsLocator` | CargoEcosystem |
| `scala.rs` | 307 | `ScalaExternalsLocator` (shim) | MavenEcosystem |
| `swift.rs` | 461 | `SwiftExternalsLocator` | SpmEcosystem |
| `typescript.rs` | 756 | `TypeScriptExternalsLocator` | NpmEcosystem |
| `zig.rs` | 116 | `ZigExternalsLocator` | ZigPkgEcosystem |

### A2. `indexer/manifest/` — 20 files (to fold into ecosystems)

`cargo.rs` → CargoEcosystem · `clojure.rs` → MavenEcosystem · `composer.rs` → ComposerEcosystem · `description.rs` → CranEcosystem · `gemfile.rs` → RubygemsEcosystem · `gleam.rs` → HexEcosystem · `go_mod.rs` → GoModEcosystem · `gradle.rs` → MavenEcosystem · `maven.rs` → MavenEcosystem · `mix.rs` → HexEcosystem · `npm.rs` → NpmEcosystem · `nuget.rs` → NugetEcosystem · `opam.rs` → OpamEcosystem · `pubspec.rs` → PubEcosystem · `pyproject.rs` → PypiEcosystem · `rockspec.rs` → LuarocksEcosystem · `sbt.rs` → MavenEcosystem · `swift_pm.rs` → SpmEcosystem · `zig_zon.rs` → ZigPkgEcosystem · `mod.rs` — deleted.

### A3. `languages/<lang>/externals.rs` — 43 files, 2,898 lines (to delete)

**Tier A (≤50 loc, pure const arrays):** `ada` (70), `bash` (35), `csharp` (3), `dockerfile` (92)*, `fsharp` (3), `gdscript` (38), `gleam` (26), `haskell` (24), `java` (5), `matlab` (34), `ocaml` (35), `php` (35), `rust_lang` (3), `scala` (4), `scss` (3), `svelte` (19), `typescript` (15), `vue` (11). (*asterisk = borderline; audit in Phase 0.)

**Tier B (≥50 loc, gated on Phase 5 per language):** `c_lang` (232), `javascript` (282), `clojure` (191), `kotlin` (159 — Android SDK logic moved to AndroidSdk ecosystem), `erlang` (137), `groovy` (129), `prolog` (120), `robot` (120), `fortran` (111), `nim` (91), `bicep` (88), `dockerfile` (92), `zig` (86), `nix` (68), `hcl` (69), `odin` (63), `starlark` (60), `elixir` (59), `ruby` (55), `powershell` (51), `perl` (51), `lua` (49).

### A4. `languages/<lang>/builtins.rs` — 48 files, 12,021 lines (to become `keywords.rs`)

Top by size: `c_lang` 1528 · `vba` 858 · `clojure` 631 · `elixir` 581 · `gdscript` 548 · `rust_lang` 516 · `kotlin` 434 · `ruby` 428 · `fsharp` 389 · `matlab` 277 · `python` 276 · `php` 264 · `scala` 263 · `erlang` 248 · `typescript` 228 · `java` 224 · `puppet` 218 · `robot` 216 · `swift` 209 · `starlark` 202 · `scss` 192 · `hcl` 175 · `haskell` 172 · `vue` 161 · `r_lang` 158 · `nix` 150 · `dart` 149 · `zig` 147 · `perl` 140 · `prolog` 135 · `bash` 133 · `fortran` 133 · `lua` 130 · `bicep` 127 · `ocaml` 122 · `csharp` 111 · `groovy` 104 · `powershell` 108 · `pascal` 108 · `cobol` 104 · `svelte` 87 · `ada` 85 · `gleam` 83 · `odin` 82 · `go` 74 · `nim` 60 · `dockerfile` 39.

### A5. Shared-resolver hardcoded lists

- `engine.rs:2482` — `fn test_framework_globals(dep: &str)`, ~109 names across jest/vitest/mocha/chai/ava/jasmine/bun-types.
- `engine.rs:786` — `build_test_globals_union` call site.
- `engine.rs:788` — `build_test_globals_by_pkg` call site.
- `engine.rs:1445-1450` — `Some("test_framework")` origin tag.
- `heuristic.rs` — no hardcoded symbol arrays.
- `framework_globals.rs` — already down to `is_test_file()`.

### A6. Per-language resolver inline matcher density (Phase 9 scope)

`cmake/resolve.rs` 72 · `python/resolve.rs` 51 · `typescript/resolve.rs` 46 · `rust_lang/resolve.rs` 44. Others below 30 each.

### A7. Existing environment probes (prior art for stdlib ecosystems)

- `languages/kotlin/externals.rs:74` — `discover_android_sdk_roots()`. Probes `$ANDROID_HOME`, scans `platforms/`, extracts `android.jar` stubs to cache. **Template for AndroidSdk + KotlinStdlib ecosystems in Phase 5.**
- `indexer/externals/python.rs:305-437` — site-packages discovery: env var override (`BEARWISDOM_PYTHON_SITE_PACKAGES`) → `.venv` / `venv` / `.env` scan → `PYTHONHOME`. Template for CpythonStdlib probe.
- `indexer/externals/mod.rs:254-268` — `maven_local_repo()`: `BEARWISDOM_JAVA_MAVEN_REPO` env or `~/.m2/repository`. Template for MavenEcosystem + JdkSrc.

### A8. Re-audit commands

```bash
# Inventory counts
find crates/bearwisdom/src/indexer/externals -name '*.rs' | xargs wc -l
find crates/bearwisdom/src/indexer/manifest -name '*.rs' | xargs wc -l
find crates/bearwisdom/src/languages -name 'externals.rs' | xargs wc -l
find crates/bearwisdom/src/languages -name 'builtins.rs' | xargs wc -l

# Trait surface
grep -n "fn .*()" crates/bearwisdom/src/languages/mod.rs | head -40

# Shared-resolver hardcoded lists
grep -n "test_framework_globals\|build_test_globals\|AMBIGUITY_LIMIT" \
  crates/bearwisdom/src/indexer/resolve/*.rs

# Per-language resolver inline string density
for f in crates/bearwisdom/src/languages/*/resolve.rs; do
  n=$(grep -cE '"[a-zA-Z_][a-zA-Z_0-9]*"' "$f")
  [ "$n" -gt 30 ] && echo "$f: $n"
done

# After refactor: confirm deletions
find crates/bearwisdom/src/languages -name 'externals.rs'        # expect empty
find crates/bearwisdom/src/languages -name 'builtins.rs'          # expect empty
test -d crates/bearwisdom/src/indexer/externals && echo STILL     # expect gone
test -d crates/bearwisdom/src/indexer/manifest  && echo STILL     # expect gone
grep -rn "externals_locator\|test_framework_globals" crates/      # expect empty
```
