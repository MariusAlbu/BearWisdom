# Code Review #2 — BearWisdom Consolidated Findings

**Date**: 2026-04-06
**Branch**: feat/resolution-engine (after CR1 complete — 118 files, +5607/-652 lines)
**Reviewers**: 3x Claude code-reviewer agents (architecture, performance, code quality)
**Codex pass**: Failed on all 3 (ENOBUFS / JSONL parse errors)
**Codebase**: ~145K lines Rust, 535 files, ~2200 tests

---

## CRITICAL

### 1. QueryError enum is defined but unused — all query functions still return anyhow::Result
**Type**: Architecture | **Found by**: Architecture + Quality reviewers

Every query function returns `anyhow::Result<T>`. The new `QueryError` with `NotIndexed`, `NotFound`, `DatabaseBusy`, `Internal` is defined, has From impls, re-exported from lib.rs — but zero query functions return `QueryResult<T>`. MCP server converts everything through anyhow into opaque strings.

**Recommendation**: Migrate query functions to return `QueryResult<T>` incrementally. Start with MCP-exposed ones.

---

### 2. N+1 SQL per chunk in hybrid search RRF scoring loop
**Type**: Performance | **Found by**: Performance reviewer

`hybrid_search` calls `chunk_file_path()` + `fetch_chunk_meta()` individually for every candidate chunk (lines 136-161). For N candidates: 2N individual queries in the hot search path. With 40 FTS + 40 vector results = 80-160 queries per search.

**Recommendation**: Batch-fetch all chunk metadata in a single query using `WHERE cc.id IN (...)`.

---

## HIGH

### 3. MCP bw_blast_radius ignores user-supplied max_results parameter
**Type**: Correctness | **Found by**: Architecture + Quality reviewers

`BlastRadiusParams` declares `max_results: Option<u32>` but the handler hardcodes `500`. User-supplied value silently ignored.

**Recommendation**: Replace `500` with `params.max_results.unwrap_or(500).min(5000).max(1)`.

---

### 4. Correlated UPDATE scans entire symbols table for incoming_edge_count
**Type**: Performance | **Found by**: Performance reviewer

```sql
UPDATE symbols SET incoming_edge_count = (
    SELECT COUNT(*) FROM edges WHERE target_id = symbols.id
)
```
One correlated subquery per symbol row. For 30K symbols, O(S * log E).

**Recommendation**: Rewrite as batch: compute counts in temp table, then JOIN-update.

---

### 5. Excessive cloning in SymbolIndex::build — 630K+ String allocations for 30K symbols
**Type**: Performance | **Found by**: Performance reviewer

Inner loop clones `SymbolInfo` 3x per symbol (into by_name, by_qname, by_file), each with 7 String fields. Plus `sorted_qnames` clones the entire by_qname map again.

**Recommendation**: Use `Arc<str>` or string interning for file_path. Consider `BTreeMap` for by_qname (eliminates sorted_qnames clone). Arena allocation for SymbolInfo.

---

### 6. Database delegation methods bypassed — 47+ direct db.conn accesses vs ~24 delegation uses
**Type**: Architecture | **Found by**: Architecture reviewer

Metrics layer is blind to most queries.

**Recommendation**: Make `conn` private, force all access through delegation methods.

---

### 7. DbPool::enable_metrics() creates metrics instance that is never stored
**Type**: Architecture | **Found by**: Architecture + Quality reviewers

Creates `Arc<QueryMetrics>`, returns it, never connects to pool. Dead code in practice.

**Recommendation**: Remove the method (metrics always enabled at construction).

---

### 8. unsafe transmute of sqlite3_vec_init — return code not checked
**Type**: Safety | **Found by**: Architecture + Quality reviewers

`std::mem::transmute` for FFI call, rc logged but never checked. UB risk if sqlite-vec changes signature.

**Recommendation**: Assert rc == SQLITE_OK. Document expected signature.

---

### 9. Per-file correlated DELETE in blast-radius cleanup — 3M queries for M files
**Type**: Performance | **Found by**: Performance reviewer

Three sequential SQL queries per affected file (SELECT id, DELETE unresolved_refs, DELETE external_refs).

**Recommendation**: Collect all file_ids, batch the deletes.

---

### 10. open_with_vec() is a dead alias identical to open()
**Type**: Architecture | **Found by**: Architecture reviewer

6 callers still use it. sqlite-vec is always available.

**Recommendation**: Remove and migrate callers.

---

### 11. std::cell::OnceCell in Database is !Sync
**Type**: Architecture | **Found by**: Architecture reviewer

Works today because Connection is !Sync, but `std::sync::OnceLock` is zero-cost and forward-compatible.

**Recommendation**: Replace with `OnceLock`.

---

## MEDIUM

### 12. Per-file chunk ID collection via individual queries in hybrid search
**Type**: Performance | **Found by**: Performance reviewer

For each FTS-matching file, `chunk_ids_for_file_path` runs a separate JOIN. 20 files = 20 queries before scoring begins.

**Recommendation**: Batch with `WHERE f.path IN (...)`.

---

### 13. Query cache double-serialization in MCP path
**Type**: Architecture | **Found by**: Architecture reviewer

Cache stores serialized JSON. MCP path would deserialize then re-serialize.

**Recommendation**: MCP server check cache directly, return cached JSON.

---

### 14. Unbounded LIKE scan in smart_context seed fallback
**Type**: Performance | **Found by**: Performance reviewer

`WHERE lower(s.name) LIKE '%word%'` — full table scan per keyword. 5 keywords × 30K symbols = 5 sequential scans.

**Recommendation**: Use FTS5 trigram index. Short-circuit when prior strategies found enough results.

---

### 15. Eight sequential COUNT(*) queries in read_stats
**Type**: Performance | **Found by**: Performance reviewer

8 individual `SELECT COUNT(*) FROM <table>` queries.

**Recommendation**: Combine into single query with subselects.

---

### 16. Nested generic parsing uses sig.find for angle brackets — fragile
**Type**: Correctness | **Found by**: Architecture reviewer

Finds first close bracket, not balanced one. `Map<String, List<Int>>` extracts wrong params.

**Recommendation**: Use bracket-depth counter.

---

### 17. CLI `bw sql` executes arbitrary SQL without write guards
**Type**: Safety | **Found by**: Quality reviewer

User or LLM agent could `DROP TABLE symbols`.

**Recommendation**: Open connection read-only, or add `--allow-write` flag.

---

### 18. Mutex::lock().unwrap() in pool hot path — poison cascade risk
**Type**: Safety | **Found by**: Quality reviewer

If any thread panics while holding lock, all subsequent pool ops panic.

**Recommendation**: Use `.lock().unwrap_or_else(|e| e.into_inner())`.

---

### 19. from_str inherent methods shadow std::str::FromStr trait
**Type**: Architecture | **Found by**: Quality reviewer

SymbolKind/EdgeKind/Visibility have inherent `from_str` that wraps `.parse().ok()`.

**Recommendation**: Remove inherent methods. Callers use `.parse().ok()`.

---

### 20. augment_from_db loads entire symbols table even when most already in index
**Type**: Performance | **Found by**: Performance reviewer

No WHERE filter to exclude already-parsed files.

**Recommendation**: Add `WHERE f.path NOT IN (...)` filter.

---

## LOW

### 21. Per-file deletion loop without transaction
Write path: individual DELETE per file without explicit transaction.

### 22. Double file traversal in parse_file
Content read + metadata stat are separate syscalls.

### 23. invalidate_files always clears entire query cache
Fine-grained invalidation not implemented.

### 24. Silent .ok() on external_refs INSERT
Schema bugs or disk-full silently dropped.

### 25. lib.rs header references stale LSP design goals
Should reflect current "replace LSP" direction.

### 26. PoolGuard uses Option<Database> — ManuallyDrop more idiomatic

### 27. emit_chain_type_ref belongs in shared utility, not languages/mod.rs

### 28. tokio full feature in core library (only needed by gated lsp module)

### 29. expect() in MCP background indexing task — produces panic backtrace instead of error

---

## Strengths

- **Plugin system** — 59 plugins, clean trait dispatch, generic fallback
- **Resolution engine boundaries** — SymbolLookup decouples resolvers from index
- **Slim-by-default QueryOptions** — right for LLM consumers
- **DROP+CREATE for full reindex** — O(1) vs O(n log n)
- **mtime+size fast path** in changeset detection
- **Temp table pattern** for blast-radius queries
- **Covering indexes** — well-designed for common query patterns
- **FxHashMap for SymbolIndex** — faster hash for string keys
- **prepare_cached in write pipeline** — avoids repeated SQL parsing
- **Connection pool** — clean RAII with shared cache/metrics
