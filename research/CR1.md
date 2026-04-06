# Code Review #1 — BearWisdom Consolidated Findings

**Date**: 2026-04-06
**Reviewers**: 3x Claude code-reviewer agents (architecture focus, performance focus, general architecture)
**Codex pass**: Failed on all 3 (ENOBUFS — working tree diff too large for runtime buffer)
**Codebase**: ~140K lines Rust, 494 files, ~2100 tests

---

## All Items Resolved

### CRITICAL (1–5)
- **1** Unified write pipeline
- **2** mtime pre-filter
- **3** Batch smart_context
- **4** Bulk chunking
- **5** Scoped edge query

### HIGH (6–11)
- **6** Web crate DbPool
- **7** DB encapsulation + metrics + caching
- **8** Connector architecture
- **9** Incremental SymbolIndex completeness
- **10** RefCache moved to DbPool (Arc<Mutex>)
- **11** Symbol table load scoping (simplified, full load still needed)

### MEDIUM (12–19)
- **12** QueryCache wired
- **13** QueryError enum added (blast_radius migrated as proof-of-concept)
- **14** MCP tool boilerplate extracted into run_tool helper
- **15** LanguagePlugin + LanguageResolver sync — deferred to resolver expansion work
- **16** find_dependent_files + find_newly_resolvable_files batched with temp tables
- **17** SHA-256 computed from raw bytes (read→hash→String)
- **18** strum derives for SymbolKind/EdgeKind/Visibility with round-trip tests
- **19** Blast radius CTE capped with max_results (default 500) + truncated flag

### LOW (20–27)
- **20** LSP + bridge modules gated behind `#[cfg(feature = "lsp")]`
- **21** stop_words() → LazyLock static
- **22** primitives_set_for_language returns HashSet<&'static str>
- **23** has_vec_extension() cached via OnceCell
- **24** default_registry() → LazyLock singleton
- **25** dirs crate unified to v6 workspace-wide
- **26** INSERT ... RETURNING id (eliminates per-file SELECT round-trip)
- **27** HashSet for chunk dedup in hybrid search

---

## Strengths (unchanged)

- **LanguagePlugin trait + registry** — clean plugin architecture
- **Two-tier resolution** — engine-first, heuristic-fallback with confidence scoring
- **Slim-by-default `QueryOptions`** — right for LLM consumers
- **DbPool** — simple, correct RAII with WAL + busy_timeout
- **Covering indexes** — deliberate query planning
- **DROP + CREATE for full reindex** — O(1) vs O(n log n)
- **FTS5 with content tables + triggers** — no data duplication
- **Test coverage** — ~2100 tests with real-world fixture projects
