<p align="center">
  <img src="assets/logo.png" alt="BearWisdom" width="280" />
</p>

<h1 align="center">BearWisdom</h1>

<p align="center">
  <strong>Structural code intelligence that's <em>un-bear-ably</em> fast.</strong><br>
  31 languages. Cross-framework graph. Hybrid search. One index.
</p>

<p align="center">
  <a href="https://github.com/MariusAlbu/BearWisdom/actions/workflows/ci.yml"><img src="https://github.com/MariusAlbu/BearWisdom/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
  <a href="https://github.com/MariusAlbu/BearWisdom/releases"><img src="https://img.shields.io/github/v/release/MariusAlbu/BearWisdom?include_prereleases&label=release" alt="Release" /></a>
</p>

<p align="center">
  <a href="#features">Features</a> &middot;
  <a href="#quick-start">Quick Start</a> &middot;
  <a href="#web-explorer">Web Explorer</a> &middot;
  <a href="#cli-reference">CLI Reference</a> &middot;
  <a href="#architecture">Architecture</a> &middot;
  <a href="#mcp-server">MCP Server</a> &middot;
  <a href="#benchmarks">Benchmarks</a>
</p>

---

## What Is This?

BearWisdom is a **code intelligence engine** that builds a structural understanding of your entire codebase — symbols, edges, call hierarchies, concepts, and cross-language flows — and makes it searchable in milliseconds.

It parses your code with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), stores everything in a local SQLite graph database, and exposes the results through a CLI, MCP server, web UI, and Claude Code agent. Think of it as a bear that hibernated with your codebase and woke up knowing where everything is.

> **Why "BearWisdom"?** Because understanding code shouldn't require *grizzly* effort. Let the bear do the heavy lifting.

## Features

### The Bear Necessities

- **31 languages** — C#, TypeScript, JavaScript, Python, Rust, Go, Java, C/C++, Ruby, PHP, Kotlin, Swift, Scala, Haskell, Elixir, Dart, Lua, R, HTML, CSS, JSON, YAML, Bash, SQL, Markdown, XML, Dockerfile, and more
- **Structural graph** — symbols, edges (calls, inherits, implements, type_ref, instantiates), concepts, and annotations stored in SQLite
- **5-priority resolver** — namespace imports, scope analysis, qualified name matching, file-path correlation, and kind-based inference
- **Cross-framework connectors** — Spring Boot, Django, .NET DI, EF Core, gRPC, GraphQL, Electron IPC, Tauri IPC, React/Zustand, and HTTP API routes
- **Smart context** — describe a task in natural language and get the ranked symbols and files most relevant to it, ready for LLM context windows
- **Full trace** — end-to-end execution flow tracing that walks both the call graph and cross-framework flow edges (DI, HTTP, events) from any symbol or auto-detected entry points
- **Flow visualization** — Sankey diagram in the web explorer showing architecture flows with click-to-pin highlighting, depth control, and symbol-specific tracing

### Search That's Barely Believable

- **FTS5 symbol search** — BM25-ranked full-text search across all symbol names and qualified names
- **Fuzzy finder** — nucleo-powered fuzzy matching for files (Ctrl+P) and symbols (Ctrl+T)
- **Content search** — FTS5 trigram index for substring search across file content
- **Grep** — gitignore-aware regex/literal search with scope filtering
- **Hybrid search** — FTS5 + ONNX vector embeddings fused via Reciprocal Rank Fusion
- **Semantic search** — natural language queries via CodeRankEmbed embeddings

### Analysis That's Polar-izing

- **Architecture overview** — language breakdown, hotspot detection, entry point discovery
- **Blast radius** — "if I change X, what breaks?" via recursive CTE graph traversal
- **Call hierarchy** — incoming and outgoing call chains with edge provenance
- **Full trace** — end-to-end execution tracing from entry points through call chains and cross-framework flow edges (DI, HTTP, events, IPC)
- **Smart context** — natural language task description to ranked symbols for LLM context windows (multi-strategy seeding: FTS5 raw, per-keyword, LIKE fallback)
- **Concept discovery** — automatic namespace grouping with member assignment
- **Subgraph export** — filtered graph export for D3/Cytoscape visualization

### Ways to Interact

| Interface | Description |
|-----------|-------------|
| **`bw` CLI** | 30+ JSON-output commands for scripts and agents |
| **Web Explorer** | React + D3 force-directed knowledge graph with Sankey flow diagrams |
| **MCP Server** | Model Context Protocol server for Claude and other LLMs |
| **Claude Code Agent** | Conversational subagent that wraps the CLI |

## Quick Start

### Prerequisites

- **Rust** 1.75+ (edition 2021)
- **Node.js** 18+ (for the web explorer, optional)
- **ONNX Runtime** (required for AI search / semantic embeddings)
  - Install via Python: `pip install onnxruntime` and set `ORT_DYLIB_PATH` to the `onnxruntime` shared library inside that package, or download a standalone release from [github.com/microsoft/onnxruntime/releases](https://github.com/microsoft/onnxruntime/releases) and point `ORT_DYLIB_PATH` at `onnxruntime.dll` / `libonnxruntime.so` / `libonnxruntime.dylib`
  - Without `ORT_DYLIB_PATH` the engine starts normally but `bw hybrid`, `bw embed`, and AI Search in the web UI will return an error

### Build

```bash
# Clone the repo
git clone https://github.com/MariusAlbu/BearWisdom.git
cd BearWisdom

# Build all crates
cargo build --release

# The CLI binary is at:
./target/release/bw --help
```

### Index a Project

```bash
# Full index with concept discovery + automatic embedding (if ORT_DYLIB_PATH is set)
bw open /path/to/your/project

# Check index stats
bw status /path/to/your/project
```

The index is stored at `<project>/.bearwisdom/index.db` — a single SQLite file. Add `.bearwisdom/` to your project's `.gitignore`.

For AI search, `bw open` automatically computes CodeRankEmbed embeddings after indexing when `ORT_DYLIB_PATH` is set. You can also run embedding standalone:

```bash
# Download the CodeRankEmbed model first (one time)
# Place it at <project>/models/CodeRankEmbed  OR  ~/.bearwisdom/models/CodeRankEmbed

# Compute embeddings independently
bw embed /path/to/your/project
```

### Search

```bash
# Find symbols by name
bw search-symbols /path/to/project "ProductService"

# Fuzzy find files (like Ctrl+P)
bw fuzzy-files /path/to/project "ProdServ"

# Fuzzy find symbols (like Ctrl+T) — yes, it's *fuzzy*, like a bear
bw fuzzy-symbols /path/to/project "GetById"

# Grep across files
bw grep /path/to/project "TODO" --case-insensitive

# Content search (trigram, min 3 chars)
bw search-content /path/to/project "repository"

# Hybrid search (FTS + embeddings when model is available)
bw hybrid /path/to/project "authentication middleware"
```

### Navigate

```bash
# Go to definition
bw definition /path/to/project "ProductService"

# Find all references
bw references /path/to/project "IProductRepository"

# List symbols in a file
bw file-symbols /path/to/project "Services/ProductService.cs"

# Full symbol detail
bw symbol-info /path/to/project "ProductService"
```

### Analyze

```bash
# Architecture overview
bw architecture /path/to/project

# Blast radius — what breaks if I change this?
bw blast-radius /path/to/project "Product" --depth 3

# Who calls this?
bw calls-in /path/to/project "GetById"

# What does this call?
bw calls-out /path/to/project "PlaceOrder"

# Cross-language flow trace
bw trace-flow /path/to/project "Controllers/OrderController.cs" 45
```

### Concepts

```bash
# Discover namespace concepts automatically
bw discover-concepts /path/to/project

# List concepts
bw concepts /path/to/project

# Show concept members
bw concept-members /path/to/project "MyApp.Services"

# Export graph filtered by concept
bw export-graph /path/to/project --filter "@MyApp.Services"
```

## Web Explorer

BearWisdom includes a web-based knowledge graph explorer. It's a React + D3 application that lets you visually explore the structural graph.

### Run

```bash
# Build the frontend (one time)
cd web && npm install && npm run build && cd ..

# Start the server
cargo run -p bearwisdom-web --release -- --static-dir web/dist

# Open http://localhost:3030
```

### What You Can Do

- **Browse** your filesystem and select a project to index
- **Explore** the force-directed knowledge graph — drag, zoom, pan, hover to highlight connections
- **Filter by concept** — click a concept in the sidebar to isolate that subgraph
- **Search 6 ways** — Symbols, Fuzzy, Files, Content, Grep, and AI Search tabs
- **Enable AI Search** — the web UI has an "Enable AI Search" button that triggers embedding computation on demand (requires `ORT_DYLIB_PATH` set in the server environment)
- **Inspect symbols** — click a node to see its signature, documentation, incoming/outgoing calls
- **View source** — file/content/grep results open a full code viewer with line highlighting
- **Resize the detail panel** — drag the left edge to make room for code review
- **Flow tab** — Sankey diagram showing end-to-end execution flows across the codebase. Trace from specific symbols or auto-detected entry points. Click nodes to pin paths, hover to explore connections. Depth slider controls trace depth. Node colors indicate symbol kind (class, method, interface)

### Development

```bash
# Backend (auto-rebuilds on change)
cargo run -p bearwisdom-web -- --port 3030

# Frontend dev server (hot reload, proxies /api to :3030)
cd web && npm run dev
# Open http://localhost:5173
```

## CLI Reference

All commands output JSON to stdout. Envelope: `{"ok": true, "data": {...}}` or `{"ok": false, "error": "..."}`.

Global flag: `--full` restores verbose output (signatures, doc comments, children).

| Command | Description |
|---------|-------------|
| `bw open <path>` | Full index + concept discovery + post-index embedding |
| `bw status <path>` | Index stats (read-only) |
| `bw embed <path>` | Compute CodeRankEmbed embeddings standalone |
| `bw architecture <path>` | Language stats, hotspots, entry points |
| `bw search-symbols <path> <query>` | FTS5 symbol search |
| `bw fuzzy-files <path> <pattern>` | Fuzzy file finder |
| `bw fuzzy-symbols <path> <pattern>` | Fuzzy symbol finder |
| `bw search-content <path> <query>` | FTS5 trigram content search |
| `bw grep <path> <pattern>` | Regex/literal grep |
| `bw hybrid <path> <query>` | Hybrid FTS + vector search |
| `bw definition <path> <symbol>` | Go to definition |
| `bw references <path> <symbol>` | Find all references |
| `bw file-symbols <path> <file>` | Symbols in a file |
| `bw symbol-info <path> <symbol>` | Full symbol detail |
| `bw blast-radius <path> <symbol>` | Impact analysis |
| `bw calls-in <path> <symbol>` | Incoming call hierarchy |
| `bw calls-out <path> <symbol>` | Outgoing call hierarchy |
| `bw trace-flow <path> <file> <line>` | Cross-language flow |
| `bw full-trace <path> [symbol]` | End-to-end execution trace (call graph + flow edges) |
| `bw smart-context <path> <task>` | Smart context selection for LLM prompts |
| `bw investigate <path> <symbol>` | Combined deep-dive (symbol info + callers + callees + blast radius) |
| `bw complete-at <path> <file> <line>` | Scope-aware symbol completion |
| `bw diagnostics <path> <file>` | File diagnostics (unresolved refs, low confidence edges) |
| `bw quality-check --baseline <file>` | Regression testing against quality baseline |
| `bw import-scip <path> --scip <file>` | Import SCIP index for high-confidence edges |
| `bw concepts <path>` | List concepts |
| `bw discover-concepts <path>` | Auto-discover concepts |
| `bw concept-members <path> <concept>` | Concept members |
| `bw export-graph <path>` | Graph export (JSON) |

## Architecture

See the [Architecture Diagram](docs/architecture.html) for a visual overview.

```
bearwisdom/                    Core library — parser, indexer, query, search, bridge
  src/
    parser/                    Tree-sitter extractors (31 languages)
    indexer/                   Full + incremental indexing
    query/                     Architecture, blast radius, call hierarchy, full trace,
                               smart context, investigate, diagnostics, completion,
                               concepts, search, subgraph, definitions
    search/                    Grep, FTS5, fuzzy, hybrid, embeddings, vector store
    bridge/                    SCIP import, background enrichment
    connectors/                Cross-framework edge detection (Spring, Django, EF Core, etc.)
    db/                        SQLite schema, database management

bearwisdom-cli/                CLI binary (bw) — 30+ JSON commands
bearwisdom-mcp/                MCP server (bw-mcp) — tool registration
bearwisdom-web/                Web server (bw-web) — Axum HTTP + static files + Sankey flows
bearwisdom-profile/            Language detection, project scanning
bearwisdom-bench/              Benchmark harness

benchmarks/                    LLM benchmark runner (bw-bench) — API-based task evaluation
web/                           React + D3 frontend (Vite + TypeScript)
tests/                         Integration test suite
agents/                        Claude Code agent definitions
```

### Bundled Dependencies

**sqlite-vec** is statically linked into the BearWisdom binary. No `SQLITE_VEC_PATH` environment variable or external `.dll`/`.so` is needed — vector search works out of the box.

**ONNX Runtime** is loaded dynamically at runtime via the `load-dynamic` feature. Set `ORT_DYLIB_PATH` to the path of the shared library before running any embedding command. sqlite-vec handles storage; ONNX Runtime handles inference.

### Database Schema

The SQLite database stores:

| Table | Purpose |
|-------|---------|
| `files` | Indexed files with path, language, content hash, and timestamp |
| `symbols` | Extracted symbols (name, qualified_name, kind, line, signature, doc_comment) |
| `edges` | Directed relationships (calls, inherits, implements, type_ref, instantiates) with confidence |
| `unresolved_refs` | References pending LSP/SCIP resolution |
| `imports` | Import/using directives per file, used by the 5-priority resolver |
| `routes` | HTTP route endpoints extracted by framework connectors |
| `db_mappings` | EF Core entity-to-table mappings |
| `symbols_fts` | FTS5 virtual table for BM25-ranked symbol search |
| `annotations` | Free-form markdown notes attached to symbols |
| `concepts` | Namespace groupings with auto_pattern for membership matching |
| `concept_members` | Symbol-to-concept assignments (manual and auto) |
| `lsp_edge_meta` | LSP-resolved edge provenance |
| `fts_content` | FTS5 trigram virtual table for file content search |
| `code_chunks` | AST-aware chunks aligned to symbol boundaries, used for embeddings |
| `flow_edges` | Cross-language flow edges (TS→C#, gRPC client→server, etc.) |
| `search_history` | Recent and saved searches with query type and scope |

## MCP Server

BearWisdom ships an MCP (Model Context Protocol) server that exposes all capabilities as tools for Claude Code and other LLM agents. The server indexes the project in the background on startup and makes all query tools available immediately.

### Setup

```bash
# Build the MCP server (release recommended — it runs as a long-lived process)
cargo build --release -p bearwisdom-mcp

# Register it for a specific project
./target/release/bw-mcp register --project /path/to/your/project
```

This writes a `bearwisdom` entry into `<project>/.mcp.json`. Next time you open Claude Code in that project directory, the MCP server starts automatically.

### Manual usage

```bash
# Run standalone (indexes current directory by default)
./target/release/bw-mcp

# Run for a specific project
./target/release/bw-mcp --project /path/to/your/project

# Unregister from a project
./target/release/bw-mcp unregister --project /path/to/your/project
```

The server communicates over stdio using JSON-RPC (MCP protocol). On startup it runs a full index in the background — tool calls during indexing will block briefly until the current batch finishes.

## Claude Code Agent

The `agents/bearwisdom.md` agent lets you interrogate any codebase conversationally:

```
> /bearwisdom

You: What's the architecture of this project?
Bear: Running bw architecture... [shows language stats, hotspots, entry points]

You: Who calls ProductService?
Bear: Running bw calls-in ProductService... [shows call hierarchy]

You: What would break if I change the Order model?
Bear: Running bw blast-radius Order --depth 3... [shows impact analysis]
```

## Project Structure

| Crate | Type | Description |
|-------|------|-------------|
| `bearwisdom` | lib | Core engine — 31-language parser, graph DB, hybrid search |
| `bearwisdom-cli` | bin (`bw`) | CLI with 30+ JSON commands |
| `bearwisdom-mcp` | bin (`bw-mcp`) | MCP server for LLM agents |
| `bearwisdom-web` | bin (`bw-web`) | Axum HTTP server + React UI |
| `bearwisdom-profile` | lib | Language detection, project scanning |
| `bearwisdom-bench` | bin | Index benchmarks |
| `bw-bench` | bin | LLM benchmark runner (in `benchmarks/`) |

## Benchmarks

BearWisdom includes a benchmark harness (`bw-bench`) that compares code intelligence quality across three conditions:

- **MCP**: BearWisdom tools via MCP protocol
- **CLI**: BearWisdom tools via `bw` CLI commands
- **Native**: Standard Read/Grep/Glob tools only

Tested across 4 projects (eShop, SimplCommerce, go-gitea, react-calcom) with 10 tasks each covering 6 categories: symbol lookup, cross-file references, call hierarchy, impact analysis, architecture overview, and code navigation.

```bash
# Generate tasks from a test project
bw-bench generate --project /path/to/project --output bench-results/tasks.json

# Run benchmarks (all conditions)
bw-bench run --tasks bench-results/tasks.json --model claude-sonnet-4-6 --output bench-results/

# Generate report
bw-bench report --results bench-results/

# Full pipeline (generate + run + report)
bw-bench full --project /path/to/project --model claude-sonnet-4-6 --output bench-results/
```

Metrics: precision, recall, F1, efficiency (penalizes tool call count), token usage. Requires `ANTHROPIC_API_KEY`.

## License

MIT License. See [LICENSE-MIT](LICENSE-MIT) for details.

---

<p align="center">
  <em>Built with the patience of a bear and the precision of tree-sitter.</em><br>
  <em>May your searches always be <strong>fuzzy</strong> in the right way.</em> 🐻
</p>
