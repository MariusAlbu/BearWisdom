# Contributing to BearWisdom

Thanks for your interest in helping the bear get smarter.

## Getting Started

```bash
git clone https://github.com/MariusAlbu/BearWisdom.git
cd BearWisdom
cargo build
cargo test --workspace
```

## Development

### Prerequisites

- Rust 1.75+ (stable)
- Node.js 18+ (for the web explorer)
- **ONNX Runtime** (required only for testing AI search features) — set `ORT_DYLIB_PATH` to the shared library path before running embedding-related tests or commands. See the README Prerequisites section for install options.

### Project Layout

| Crate | What it does |
|-------|-------------|
| `crates/bearwisdom` | Core library — parser, indexer, query engine, search |
| `crates/bearwisdom-cli` | CLI binary (`bw`) |
| `crates/bearwisdom-mcp` | MCP server (`bw-mcp`) |
| `crates/bearwisdom-web` | Web server (`bw-web`) + React UI in `web/` |
| `crates/bearwisdom-profile` | Language detection and project scanning |
| `crates/bearwisdom-bench` | Benchmark harness (`bw-bench`) |
| `tests/` | Integration tests |

### Running the Web UI in Dev Mode

```bash
# Terminal 1 — backend
cargo run -p bearwisdom-web -- --port 3030

# Terminal 2 — frontend (hot reload, proxies API to :3030)
cd web && npm install && npm run dev
```

The web frontend uses [Zustand](https://github.com/pmndrs/zustand) for state management (3 stores). It also has 6 custom hooks and 54 Vitest tests. If you're setting up a fresh clone, `npm install` in `web/` handles all dependencies.

### Code Quality

```bash
# Format
cargo fmt --all

# Lint
cargo clippy --workspace --all-targets

# Test (Rust)
cargo test --workspace

# Test (frontend)
cd web && npx vitest run

# Web build
cd web && npm run build
```

### Running the Benchmark Suite

`bw-bench` has four subcommands:

```bash
# Generate a synthetic benchmark corpus (fixtures and workload definitions)
cargo run -p bearwisdom-bench -- generate

# Run a specific benchmark workload against an existing index
cargo run -p bearwisdom-bench -- run

# Produce a human-readable report from the last run's results
cargo run -p bearwisdom-bench -- report

# Generate corpus, run all workloads, and produce a report in one step
cargo run -p bearwisdom-bench -- full
```

Use `bw-bench full` before opening a PR that touches the indexer, resolver, or search paths to confirm there are no regressions.

## Pull Requests

1. Fork the repo and create a branch from `main`
2. Make your changes
3. Run `cargo fmt`, `cargo clippy`, and `cargo test`
4. If you changed the web UI, run `npm run build` in `web/` and `npx vitest run` in `web/` to confirm all 54 frontend tests pass
5. Open a PR with a clear description of what and why

Keep PRs focused — one concern per PR. The bear prefers small, digestible meals.

## Adding Language Support

BearWisdom uses tree-sitter grammars. To add a new language:

1. Add the tree-sitter grammar to `Cargo.toml` workspace dependencies
2. Register it in `crates/bearwisdom/src/parser/languages.rs`
3. Add extraction logic in `crates/bearwisdom/src/parser/extractors/` (or use the generic extractor)
4. Add tests

## Reporting Issues

Open a GitHub issue with:
- What you expected
- What happened instead
- Steps to reproduce
- BearWisdom version (`bw --version`)

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
