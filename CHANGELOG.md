# Changelog

All notable changes to BearWisdom are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] — 2026-03-28

### Added

- **Full trace engine** (`query/full_trace.rs`) — end-to-end execution flow tracing that walks both the call graph (`edges` table) and cross-framework flows (`flow_edges` table). Traces from specific symbols or auto-detected structural roots
- **Smart context fix** — `smart_context` now works with natural language queries via multi-strategy seeding (FTS5 raw, per-keyword, LIKE fallback). Previously returned empty results for non-symbol-name queries
- **Flow visualization** — Sankey diagram in the web explorer showing architecture flows with click-to-pin highlighting, hover exploration, depth control, and symbol-specific tracing
- **CLI: `bw full-trace`** — trace execution flows from a symbol or all entry points
- **CLI: `bw diagnose`** (bench) — audit project index quality before running benchmarks
- **Web API: `/api/full-trace`** — full trace endpoint for the web UI
- **Web API: `/api/flow-edges`** — flow edge listing with interleaved edge types and summary
- **Web API: `/api/trace-flow`** — wrapper for existing trace-flow functions
- **Benchmark: CLI condition** — `UseBearWisdomCli` condition that tests BearWisdom via `bw` CLI commands through Bash, complementing MCP and native-only conditions
- **Benchmark: `--strict-mcp-config`** — proper isolation between benchmark conditions (prevents project MCP configs from leaking into non-MCP conditions)
- **Integration tests** — 43 new tests in `tests/tests/commands.rs` covering all query commands against real fixture projects
- **SCIP import tests** — 5 integration tests for edge creation, confidence upgrade, idempotency
- **Quality check tests** — 6 integration tests for baseline comparison, regression detection
- **BearWisdom agent** (`agents/bearwisdom.md`) — Claude Code agent with full CLI command reference for conversational code intelligence
- **Container descent in full_trace** — classes/structs automatically expand into their methods when tracing, so entry-point classes produce meaningful call chains

### Changed

- **Benchmark sampler** — CrossFileReferences query now includes `struct` and `type_alias` kinds with `NULL` visibility support, fixing empty results for TypeScript and Go projects
- **Benchmark sampler** — ground truth deduplication across all categories (ImpactAnalysis, CallHierarchy, CrossFileReferences, ArchitectureOverview)
- **Benchmark report** — uses `Condition::all()` instead of hardcoded pairs, automatically includes any condition with data
- **Entry point detection** — `trace_from_entry_points` now uses graph topology (outgoing edges, zero incoming calls) instead of architecture overview heuristics
- **Web Flow tab** — replaced metro map with Sankey diagram. Node colors based on symbol kind (class, method, interface) instead of hardcoded layer names

### Fixed

- **smart_context** — no longer returns empty results for natural language queries like "add a new catalog item with validation"
- **Benchmark CLI condition** — `--strict-mcp-config` prevents project `.mcp.json` from injecting MCP tools into non-MCP benchmark conditions

### Infrastructure

- Test count: 885 (up from ~830)
- All benchmark results archived in `bench-archive/` (gitignored)
- `.gitignore` consolidated: all bench output under single `bench-archive/` entry

## [0.1.1] — 2026-03-26

### Added

- **sqlite-vec statically linked** — vector search works out of the box; `SQLITE_VEC_PATH` is no longer required or read
- **Post-index embedding pipeline** — `bw open` automatically computes CodeRankEmbed embeddings after indexing completes (CLI and MCP). The web UI exposes an "Enable AI Search" button that triggers embedding separately
- **`bw embed <path>`** — new standalone command to compute or refresh CodeRankEmbed embeddings without re-indexing
- **`Embedder::resolve_model_dir()`** — centralized model lookup: checks `<project>/models/CodeRankEmbed` first, then `~/.bearwisdom/models/CodeRankEmbed`
- **ONNX Runtime via `load-dynamic`** — the `onnxruntime` crate is built with `load-dynamic`, so the binary links at runtime. Set `ORT_DYLIB_PATH` to the path of `onnxruntime.dll` / `libonnxruntime.so` / `libonnxruntime.dylib`; inference batch size is 4 to stay within CPU memory budgets
- **Web UI refactored** — migrated to Zustand state management; `KnowledgeTree` component decomposed from 1,232 LOC to 104 LOC across focused sub-components; 54 Vitest tests added
- **`delete_file_vectors`** — incremental re-index now removes stale vector embeddings for deleted or modified files before re-embedding
- **Web Explorer** (`bearwisdom-web`) — React + D3 force-directed knowledge graph UI
  - Landing page with filesystem browser and indexing progress
  - 6 search modes: Symbols, Fuzzy, Files, Content, Grep, AI Search
  - Concept sidebar with filtering
  - Symbol detail panel with resizable width
  - File viewer with line highlighting for content/grep results
  - BearWisdom "Scholar's Workshop" theme (copper, teal, sage on dark brown)
- **Claude Code agent** (`agents/bearwisdom.md`) — conversational agent wrapping all CLI commands
- **Benchmark suite** (`bw-bench`) — `generate`, `run`, `report`, and `full` subcommands; CLI backend for Claude Code subscription testing and performance regression tracking
- **GitHub Pages site** (`docs/index.html`) — project landing page with real application screenshots from the eShop reference architecture
- **Architecture diagram** (`docs/architecture.html`) — visual overview of the crate dependency graph and data flow
- **Integration test suite** (`tests/`) — 54 tests across 6 suites (index pipeline, query layer, incremental, profile scan, walker, search)
- **Directory-based concept discovery** — fallback for languages without dotted namespaces (Rust, Python, Go)
- Workspace package metadata — `repository`, `homepage`, `keywords`, `categories`, `rust-version` across all crates
- `.gitattributes` with line ending normalisation and Linguist overrides
- `[profile.release]` with `strip`, `lto = "thin"`, `opt-level = 3`

### Changed

- License simplified from MIT/Apache-2.0 dual license to MIT only
- Index database location moved from `~/.bearwisdom/indexes/<hash>/` to `<project>/.bearwisdom/index.db`
- Re-indexing skipped when `.bearwisdom/index.db` already exists (returns cached stats instantly)

## [0.1.0] — 2026-03-24

Initial release. The bear wakes up.

### Core Engine (`bearwisdom`)

- **31-language parser** via tree-sitter — C#, TypeScript, JavaScript, Python, Rust, Go, Java, C/C++, Ruby, PHP, Kotlin, Swift, Scala, Haskell, Elixir, Dart, Lua, R, HTML, CSS, JSON, YAML, Bash, SQL, Markdown, XML, Dockerfile, and more
- **Dedicated extractors** for C#, TypeScript, Python, Rust, Go, Java with language-specific symbol extraction (classes, interfaces, methods, functions, enums, structs, modules, fields, properties, constructors, delegates, events)
- **Generic extractor** for all other tree-sitter grammars
- **5-priority resolver** — namespace imports, scope analysis, qualified name matching, file-path correlation, kind-based inference
- **SQLite graph database** — files, symbols, edges, concepts, annotations, FTS5 content index
- **Full indexer** — walk project, parse all files, extract symbols and edges, resolve references
- **Incremental indexer** — detect added/modified/deleted files via content hashing, re-index only changes

### Connectors

- **Spring Boot** — `@RequestMapping`, `@GetMapping`, stereotype annotations, route normalisation
- **Django** — URL patterns, class-based views, function-based views, model detection
- **.NET Dependency Injection** — `AddScoped`, `AddTransient`, `AddSingleton` registration detection
- **Entity Framework Core** — `DbSet<T>` property extraction, table name pluralisation
- **.NET Events** — integration event handlers, `INotificationHandler<T>` linking
- **gRPC** — `.proto` service/RPC parsing, server/client implementation matching
- **GraphQL** — schema type/field extraction, operation detection, resolver matching
- **Electron IPC** — `ipcMain.handle`/`ipcRenderer.invoke` channel linking
- **Tauri IPC** — `#[tauri::command]`/`invoke()` command matching
- **React Patterns** — Zustand store detection, Storybook story linking, component concept grouping
- **HTTP API** — route/handler matching across Express, FastAPI, ASP.NET, Spring, Django, Rails, Gin
- **Frontend HTTP** — `fetch`/`axios`/`requests`/`HttpClient` call-to-route matching
- **Message Queue** — Kafka, NATS, RabbitMQ producer/consumer topic linking

### Search Engine

- **FTS5 symbol search** — BM25-ranked full-text search on symbol names
- **FTS5 content search** — trigram index for substring matching in file content
- **Fuzzy finder** — nucleo-powered fuzzy matching for files and symbols
- **Grep** — gitignore-aware regex/literal search with language/directory scoping
- **Hybrid search** — FTS5 + ONNX vector embeddings fused via Reciprocal Rank Fusion
- **Semantic embeddings** — CodeRankEmbed ONNX model with int8 quantisation support
- **Vector store** — SQLite-vec for KNN similarity search
- **Content chunker** — SHA256-hashed chunks with token estimation for embedding
- **Search history** — recent and saved searches with pruning
- **Scope filtering** — language, directory, glob pattern filters for all search modes
- **Flow tracer** — cross-language dependency flow with hop limiting

### Query Layer

- **Architecture overview** — language breakdown, hotspot detection, entry point discovery
- **Blast radius** — recursive CTE graph traversal for N-hop impact analysis
- **Call hierarchy** — incoming and outgoing calls with edge kind filtering
- **Go to definition** — exact qualified name or simple name lookup
- **Find references** — all incoming edges to a symbol
- **Symbol info** — full detail including signature, doc comment, edge counts, children
- **Symbol search** — FTS5 + prefix matching with relevance scoring
- **Subgraph export** — filtered graph export (by prefix or concept) with node/edge cap
- **Concept discovery** — automatic namespace extraction from qualified names
- **Concept assignment** — pattern-based symbol-to-concept membership

### Bridge Layer

- **LSP integration** — lifecycle management for external language servers (C#, TypeScript, Python, Rust, Go, Java, C++)
- **GraphBridge** — merges LSP-resolved edges into the SQLite graph with confidence upgrading
- **BackgroundEnricher** — idle-time resolution of unresolved references via LSP hover/definition/references
- **SCIP import** — ingest SCIP index files for precise cross-reference data
- **Edge provenance** — `EdgeSource` tracking (TreeSitter, LSP, Connector, SCIP)

### CLI (`bearwisdom-cli`)

- **25 JSON commands** — open, status, architecture, search-symbols, fuzzy-files, fuzzy-symbols, search-content, grep, hybrid, file-symbols, definition, references, symbol-info, blast-radius, calls-in, calls-out, trace-flow, concepts, discover-concepts, concept-members, export-graph
- **JSON envelope** — all output as `{"ok": true, "data": ...}` or `{"ok": false, "error": "..."}`
- **SHA256 database resolution** — deterministic DB path from project root

### MCP Server (`bearwisdom-mcp`)

- **MCP tool registration** — register/unregister with Claude Code settings
- **Full tool surface** — all query and search capabilities exposed as MCP tools
- **Workspace-scoped** — one server instance per project

### Project Scanner (`bearwisdom-profile`)

- **Language detection** — file extension, filename, and alias matching for 31+ languages
- **Build exclusion** — automatic exclusion of `node_modules`, `target`, `bin`, `obj`, `.git`, etc.
- **SDK detection** — .NET SDK, Node.js, Python, Rust, Go, Java version checking
- **Package manager detection** — NuGet, npm, pip, Cargo, Go modules, Maven, Gradle
- **Test framework detection** — xUnit, NUnit, Jest, pytest, cargo test, JUnit, etc.
- **File walker** — gitignore-aware directory traversal with path normalisation

[Unreleased]: https://github.com/MariusAlbu/BearWisdom/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/MariusAlbu/BearWisdom/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/MariusAlbu/BearWisdom/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MariusAlbu/BearWisdom/releases/tag/v0.1.0
