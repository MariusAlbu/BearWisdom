// =============================================================================
// indexer/resolve/mod.rs — Reference resolution module
//
// Two-tier resolution:
//   1. Language-specific resolvers (engine.rs + rules/) — deterministic, 1.0 confidence
//   2. Heuristic fallback (heuristic.rs) — best-effort, 0.50-0.95 confidence
//
// The `resolve_and_write` function is the public entry point, called from
// `full.rs` and `incremental.rs` after symbols are written to the DB.
// =============================================================================

pub mod engine;
pub mod flow_emit;
mod heuristic;
pub mod reachability;
pub mod rules;
pub mod synthesize_dispatch;

use engine::SymbolLookup;

use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use anyhow::{Context, Result};
use engine::{build_scope_chain, ChainMiss, ImportEntry, RefContext, ResolutionEngine, SymbolIndex};
use rayon::prelude::*;
use std::collections::HashMap;
use tracing::{debug, info, warn};

use flow_emit::FlowEmission;
use crate::connectors::url_pattern;

/// Streaming-kind compatibility for an RPC pair.
///
/// Returns true when:
/// - Both sides are `None` (non-RPC, or RPC where streaming wasn't inferred).
/// - One side is `None` and the other is `Some(Unary)` (back-compat with
///   detectors that haven't been wired for streaming yet).
/// - Both sides are `Some(StreamKind::Unary)` (canonical unary pair).
/// - Both sides carry the same explicit streaming kind.
///
/// Rejects pairs where one end is unary and the other is a streaming kind,
/// or where the two ends are different streaming kinds (server-stream vs
/// client-stream, etc.).
fn streaming_kinds_compatible(
    producer: Option<flow_emit::StreamKind>,
    consumer: Option<flow_emit::StreamKind>,
) -> bool {
    use flow_emit::StreamKind;
    match (producer, consumer) {
        (None, None) => true,
        (None, Some(StreamKind::Unary)) | (Some(StreamKind::Unary), None) => true,
        (Some(p), Some(c)) => p == c,
        _ => false,
    }
}

// Per-file output buffer. Each rayon worker fills its own; the main thread
// merges + bulk-writes after the parallel section. Avoids sharing the
// rusqlite Transaction across workers (it isn't `Sync`).
#[derive(Default)]
struct FileWriteBuf {
    /// (source_id, target_id, kind, source_line, confidence, strategy)
    edges: Vec<(i64, i64, &'static str, u32, f64, &'static str)>,
    /// (source_id, target_name, kind, source_line, namespace, package_id)
    externals: Vec<(i64, String, &'static str, u32, String, Option<i64>)>,
    /// (source_id, target_name, kind, source_line, module, package_id, from_snippet)
    unresolved: Vec<(i64, String, &'static str, u32, Option<String>, Option<i64>, bool)>,
    /// Flow-edge emissions from resolver-detected patterns.
    /// Each entry: (file_path, source_line, emission).
    /// The file_path is resolved to a DB file_id during flush.
    flow_emissions: Vec<(String, u32, FlowEmission)>,
}

impl FileWriteBuf {
    fn merge(&mut self, mut other: Self) {
        self.edges.append(&mut other.edges);
        self.externals.append(&mut other.externals);
        self.unresolved.append(&mut other.unresolved);
        self.flow_emissions.append(&mut other.flow_emissions);
    }
}

/// Counters accumulated per file; reduced into the global ResolutionStats
/// after the parallel section. Excludes `chain_misses`, which are pushed
/// directly into the SymbolIndex's Mutex-protected accumulator by the
/// chain walker (already thread-safe).
#[derive(Default, Clone, Copy)]
struct FileStats {
    resolved: u64,
    engine_resolved: u64,
    unresolved: u64,
    external: u64,
}

impl FileStats {
    fn merge(&mut self, other: Self) {
        self.resolved += other.resolved;
        self.engine_resolved += other.engine_resolved;
        self.unresolved += other.unresolved;
        self.external += other.external;
    }
}

/// Bulk-flush a `FileWriteBuf` into the resolve transaction. Uses
/// multi-row VALUES inserts in fixed-size chunks so prepare_cached can
/// hit on every full chunk. Mirrors the batched write path in
/// `indexer/write.rs`.
fn flush_resolve_buf(
    tx: &rusqlite::Transaction<'_>,
    buf: &FileWriteBuf,
) -> Result<()> {
    use rusqlite::types::Value;

    // SQLITE_MAX_VARIABLE_NUMBER defaults to 32766; chunk sizes here
    // keep total vars well under that.
    const EDGE_CHUNK: usize = 256;
    const EXTERNAL_CHUNK: usize = 256;
    const UNRESOLVED_CHUNK: usize = 256;

    fn placeholders(rows: usize, cols: usize) -> String {
        let mut s = String::with_capacity(rows * (cols * 2 + 4));
        for i in 0..rows {
            if i > 0 { s.push(','); }
            s.push('(');
            for j in 0..cols {
                if j > 0 { s.push(','); }
                s.push('?');
            }
            s.push(')');
        }
        s
    }

    // Edges: (source_id, target_id, kind, source_line, confidence, strategy)
    if !buf.edges.is_empty() {
        let mut start = 0;
        while start < buf.edges.len() {
            let end = (start + EDGE_CHUNK).min(buf.edges.len());
            let rows = end - start;
            let sql = format!(
                "INSERT OR IGNORE INTO edges \
                 (source_id, target_id, kind, source_line, confidence, strategy) \
                 VALUES {}",
                placeholders(rows, 6),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 6);
            for (sid, tid, kind, line, conf, strat) in &buf.edges[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Integer(*tid));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(Value::Real(*conf));
                params.push(Value::Text((*strat).to_string()));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare batched edges insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute batched edges insert")?;
            start = end;
        }
    }

    // External refs: (source_id, target_name, kind, source_line, namespace, package_id)
    if !buf.externals.is_empty() {
        let mut start = 0;
        while start < buf.externals.len() {
            let end = (start + EXTERNAL_CHUNK).min(buf.externals.len());
            let rows = end - start;
            let sql = format!(
                "INSERT INTO external_refs \
                 (source_id, target_name, kind, source_line, namespace, package_id) \
                 VALUES {}",
                placeholders(rows, 6),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 6);
            for (sid, name, kind, line, ns, pkg) in &buf.externals[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Text(name.clone()));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(Value::Text(ns.clone()));
                params.push(match pkg {
                    Some(v) => Value::Integer(*v),
                    None => Value::Null,
                });
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare batched external_refs insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute batched external_refs insert")?;
            start = end;
        }
    }

    // Unresolved refs: (source_id, target_name, kind, source_line, module, package_id, from_snippet)
    if !buf.unresolved.is_empty() {
        let mut start = 0;
        while start < buf.unresolved.len() {
            let end = (start + UNRESOLVED_CHUNK).min(buf.unresolved.len());
            let rows = end - start;
            let sql = format!(
                "INSERT INTO unresolved_refs \
                 (source_id, target_name, kind, source_line, module, package_id, from_snippet) \
                 VALUES {}",
                placeholders(rows, 7),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 7);
            for (sid, name, kind, line, module, pkg, from_snippet) in &buf.unresolved[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Text(name.clone()));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(match module {
                    Some(s) => Value::Text(s.clone()),
                    None => Value::Null,
                });
                params.push(match pkg {
                    Some(v) => Value::Integer(*v),
                    None => Value::Null,
                });
                params.push(Value::Integer(if *from_snippet { 1 } else { 0 }));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare batched unresolved_refs insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute batched unresolved_refs insert")?;
            start = end;
        }
    }

    Ok(())
}

/// Write resolver-emitted flow edges to the `flow_edges` table.
///
/// Each emission carries a file path (not a DB id — the file may not have
/// been inserted yet when the parallel section ran). This function batches
/// a `SELECT id FROM files WHERE path IN (...)` to resolve the ids, then:
///
/// 1. Pairs `NamedChannel { Producer, .. }` ↔ `NamedChannel { Consumer, .. }` by
///    `(kind, normalized_name)` across files, with HTTP method compatibility
///    filtering.  URL patterns are normalized via `url_pattern::normalize`
///    before keying so `:id`, `<id>`, `{id}` all compare equal to `{}`.
/// 2. Pairs `DbQuery` ↔ `DbEntity` and `MigrationTarget` ↔ `DbEntity` by
///    entity/table name with case-insensitive pluralization tolerance.
///    A single `DbEntity` can accumulate many `DbQuery` and many
///    `MigrationTarget` partners — one `flow_edges` row per pair.
/// 3. Writes single-ended rows for every unpaired emission and all single-
///    ended variants (DiBinding, ConfigLookup, FeatureFlag, AuthGuard,
///    CliCommand, ScheduledJob).
fn flush_flow_emissions(
    conn: &rusqlite::Connection,
    emissions: &[(String, u32, FlowEmission)],
) -> Result<u32> {
    if emissions.is_empty() {
        return Ok(0);
    }

    // Batch-look up file_ids for all distinct paths.
    let mut path_to_id: HashMap<&str, i64> = HashMap::new();
    {
        let paths: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            emissions.iter()
                .map(|(p, _, _)| p.as_str())
                .filter(|p| seen.insert(*p))
                .collect()
        };
        for chunk in paths.chunks(256) {
            let placeholders: String = (0..chunk.len())
                .map(|i| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("SELECT id, path FROM files WHERE path IN ({placeholders})");
            let mut stmt = conn.prepare_cached(&sql)
                .context("Failed to prepare file_id lookup for flow emissions")?;
            let params: Vec<_> = chunk.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))
                .context("Failed to query file_ids for flow emissions")?;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let path: String = row.get(1)?;
                path_to_id.entry(
                    emissions.iter()
                        .find(|(p, _, _)| p == &path)
                        .map(|(p, _, _)| p.as_str())
                        .unwrap_or_default(),
                ).or_insert(id);
            }
        }
    }

    use flow_emit::{ChannelRole, FlowEmission};

    // Resolve each emission to its file_id (skip those whose file isn't in the DB).
    struct Resolved<'a> {
        file_id: i64,
        line: u32,
        emission: &'a FlowEmission,
    }
    let resolved: Vec<Resolved<'_>> = emissions.iter()
        .filter_map(|(path, line, emission)| {
            path_to_id.get(path.as_str()).map(|&file_id| Resolved { file_id, line: *line, emission })
        })
        .collect();

    // Pre-resolve every (file_id, line) appearing in `resolved` to the
    // qualified name of the innermost enclosing function/method/constructor.
    // `query::full_trace::FlowJumpMap` keys flow_edges by this name to link
    // them into the trace tree — leaving it NULL (as the previous writer did)
    // means the trace view never picks up cross-process / cross-language
    // jumps even when 70k+ flow_edges exist.
    let enclosing_qname: HashMap<(i64, u32), String> = {
        let mut stmt = conn
            .prepare_cached(
                "SELECT qualified_name FROM symbols
                 WHERE file_id = ?1
                   AND line <= ?2
                   AND (end_line IS NULL OR end_line >= ?2)
                   AND kind IN ('function', 'method', 'constructor')
                 ORDER BY line DESC
                 LIMIT 1",
            )
            .context("Failed to prepare enclosing-function lookup for flow emissions")?;
        // Fallback: nearest function/method whose declaration starts within
        // FALLBACK_LOOKAHEAD lines AFTER the emission line. Decorator
        // emissions (NestJS @Get, TypeORM @Entity) sit ON the decorator
        // line; the method/class they apply to starts on the following line,
        // outside the enclosing-range query above. The lookahead recovers
        // that owner.
        let mut following_stmt = conn
            .prepare_cached(
                "SELECT qualified_name FROM symbols
                 WHERE file_id = ?1
                   AND line > ?2
                   AND line <= ?2 + 20
                   AND kind IN ('function', 'method', 'constructor', 'class', 'struct', 'interface')
                 ORDER BY line ASC
                 LIMIT 1",
            )
            .context("Failed to prepare following-symbol lookup for flow emissions")?;
        let mut map: HashMap<(i64, u32), String> = HashMap::new();
        let mut seen: std::collections::HashSet<(i64, u32)> = Default::default();
        for r in &resolved {
            let key = (r.file_id, r.line);
            if !seen.insert(key) { continue; }
            let qn = stmt
                .query_row(rusqlite::params![r.file_id, r.line], |row| row.get::<_, String>(0))
                .or_else(|_| {
                    following_stmt.query_row(
                        rusqlite::params![r.file_id, r.line],
                        |row| row.get::<_, String>(0),
                    )
                })
                .ok();
            if let Some(qn) = qn {
                map.insert(key, qn);
            }
        }
        map
    };
    let qname_for = |file_id: i64, line: u32| -> Option<&str> {
        enclosing_qname.get(&(file_id, line)).map(String::as_str)
    };

    // -----------------------------------------------------------------------
    // Phase 1: pair NamedChannel Producer ↔ Consumer.
    //
    // Key: (edge_type_str, normalized_name).  URL patterns are normalized
    // before keying so `:id`, `<id>`, `{id}`, and `{}` all hash to the same
    // bucket.  HTTP method compatibility is checked per-pair inside the loop
    // rather than in the key, allowing `Any` to match any concrete method.
    // -----------------------------------------------------------------------
    let mut named_channel_paired: std::collections::HashSet<usize> = Default::default();

    {
        // Pre-normalize every NamedChannel name to avoid allocating inside the
        // nested loop.  Index: emission index → normalized name.
        let mut normalized_names: HashMap<usize, String> = HashMap::new();
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::NamedChannel { name, .. } = r.emission {
                if !name.is_empty() {
                    normalized_names.insert(idx, url_pattern::normalize(name));
                }
            }
        }

        // Build producer/consumer buckets keyed by (edge_type_str, normalized_name).
        let mut producers: HashMap<(&str, &str), Vec<usize>> = Default::default();
        let mut consumers: HashMap<(&str, &str), Vec<usize>> = Default::default();
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::NamedChannel { kind, role, .. } = r.emission {
                if let Some(norm) = normalized_names.get(&idx) {
                    let key = (kind.edge_type_str(), norm.as_str());
                    match role {
                        ChannelRole::Producer => producers.entry(key).or_default().push(idx),
                        ChannelRole::Consumer => consumers.entry(key).or_default().push(idx),
                    }
                }
            }
        }

        let mut pair_stmt = conn
            .prepare_cached(
                "INSERT OR IGNORE INTO flow_edges
                    (source_file_id, source_line, source_symbol,
                     target_file_id, target_line, target_symbol,
                     edge_type, protocol, http_method, url_pattern, confidence, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .context("Failed to prepare paired flow_edges INSERT")?;

        // (source_file_id, source_line, target_file_id, target_line, edge_type)
        // — guards against duplicate edges when the same emission appears
        // multiple times in the resolved list. `edge_type_str()` returns
        // `&'static str` so the lifetime is explicit here for the wildcard
        // pair helper closure that takes this set as an argument.
        let mut seen_pairs: std::collections::HashSet<(i64, u32, i64, u32, &'static str)> =
            Default::default();

        // Helper: pair one producer emission with one consumer emission. Runs
        // the same method-compat / dedup / INSERT logic the exact-match and
        // wildcard passes share. Returns Ok(()) regardless of insert outcome;
        // pair tracking is updated via the captured `seen_pairs` /
        // `named_channel_paired` sets.
        let mut pair_one = |pi: usize,
                            ci: usize,
                            url_pattern_override: Option<&str>,
                            seen_pairs: &mut std::collections::HashSet<(i64, u32, i64, u32, &'static str)>,
                            named_channel_paired: &mut std::collections::HashSet<usize>|
         -> Result<()> {
            let pr = &resolved[pi];
            let cr = &resolved[ci];
            let FlowEmission::NamedChannel { kind, method: prod_method, name, streaming: prod_streaming, .. } = pr.emission else {
                return Ok(());
            };
            let (cons_method, cons_streaming) = if let FlowEmission::NamedChannel { method, streaming, .. } = &cr.emission {
                (method.map(|m| m.as_str()), *streaming)
            } else {
                (None, None)
            };
            if !url_pattern::http_methods_compatible(
                prod_method.map(|m| m.as_str()),
                cons_method,
            ) {
                return Ok(());
            }
            // Streaming compatibility for RPC pairs: a unary producer must
            // not pair with a streaming consumer (or vice versa). `None`
            // and `Some(Unary)` are treated as equivalent so existing
            // detectors that don't set streaming still pair correctly.
            if !streaming_kinds_compatible(*prod_streaming, cons_streaming) {
                return Ok(());
            }
            let edge_type = kind.edge_type_str();
            let pair_key = (pr.file_id, pr.line, cr.file_id, cr.line, edge_type);
            if !seen_pairs.insert(pair_key) {
                named_channel_paired.insert(pi);
                named_channel_paired.insert(ci);
                return Ok(());
            }
            let url = url_pattern_override.unwrap_or(name.as_str());
            // Stream-aware metadata: when the producer or consumer carries
            // a non-unary StreamKind, record it on the edge. Unary RPC and
            // non-RPC edges get NULL metadata.
            let metadata = (*prod_streaming).or(cons_streaming).and_then(|s| {
                if s == crate::indexer::resolve::flow_emit::StreamKind::Unary {
                    None
                } else {
                    Some(s.as_str().to_string())
                }
            });
            let n = pair_stmt.execute(rusqlite::params![
                pr.file_id, pr.line, qname_for(pr.file_id, pr.line),
                cr.file_id, cr.line, qname_for(cr.file_id, cr.line),
                edge_type, kind.protocol_str(),
                prod_method.map(|m| m.as_str()),
                Some(url),
                0.9_f64,
                metadata,
            ]).context("Failed to insert paired flow_edge")?;
            if n > 0 {
                named_channel_paired.insert(pi);
                named_channel_paired.insert(ci);
            }
            Ok(())
        };

        // Exact-match pass.
        for (key, prod_idxs) in &producers {
            if let Some(cons_idxs) = consumers.get(key) {
                for &pi in prod_idxs {
                    for &ci in cons_idxs {
                        pair_one(pi, ci, None, &mut seen_pairs, &mut named_channel_paired)?;
                    }
                }
            }
        }

        // Wildcard pass: bucket key `<prefix>/*` on one role matches every
        // bucket whose key starts with `<prefix>/` on the OTHER role. Used for
        // background-job Consumer constructors (`new Worker('queue', …)` →
        // `queue/*`) that should pair with every concrete `queue.add('job', …)`
        // Producer for the same queue. Bidirectional — a wildcard Producer
        // would pair with concrete Consumers symmetrically, though no current
        // emitter produces that shape.
        //
        // Bucket keys are `(edge_type_str, normalized_name)`. Concrete and
        // wildcard entries live in different buckets, so this pass is purely
        // additive over Phase 1 exact matching.
        // Route-parameter segment matching: `/users/{}/posts` matches
        // `/users/123/posts` segment-by-segment. Segment counts must be
        // equal; `{}` segments accept any non-empty literal segment.
        fn segment_wildcard_matches(wild_key: &str, concrete_key: &str) -> bool {
            let w_segs: Vec<&str> = wild_key.split('/').collect();
            let c_segs: Vec<&str> = concrete_key.split('/').collect();
            if w_segs.len() != c_segs.len() {
                return false;
            }
            for (w, c) in w_segs.iter().zip(c_segs.iter()) {
                if w == c {
                    continue;
                }
                if *w == "{}" && !c.is_empty() {
                    continue;
                }
                return false;
            }
            true
        }

        let bucket_matches_wildcard = |wild_key: &str, concrete_key: &str| -> bool {
            // wild_key ends with `/*`, concrete_key is everything else.
            let Some(prefix) = wild_key.strip_suffix("/*") else {
                return false;
            };
            if prefix.is_empty() || concrete_key == wild_key {
                return false;
            }
            // Match `<prefix>/<anything-non-empty>` — including another
            // wildcard like `<prefix>/*/sub` if it ever appears.
            concrete_key
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
                .map_or(false, |tail| !tail.is_empty())
        };

        // Wildcard Producer × concrete Consumer.
        let wildcard_producer_keys: Vec<(&str, &str)> = producers
            .keys()
            .filter(|(_, name)| name.ends_with("/*"))
            .copied()
            .collect();
        for wkey in wildcard_producer_keys {
            let prod_idxs = producers.get(&wkey).cloned().unwrap_or_default();
            for (ckey, cons_idxs) in &consumers {
                if ckey.0 != wkey.0 {
                    continue;
                }
                if !bucket_matches_wildcard(wkey.1, ckey.1) {
                    continue;
                }
                for &pi in &prod_idxs {
                    for &ci in cons_idxs {
                        pair_one(
                            pi,
                            ci,
                            Some(ckey.1),
                            &mut seen_pairs,
                            &mut named_channel_paired,
                        )?;
                    }
                }
            }
        }

        // Wildcard Consumer × concrete Producer.
        let wildcard_consumer_keys: Vec<(&str, &str)> = consumers
            .keys()
            .filter(|(_, name)| name.ends_with("/*"))
            .copied()
            .collect();
        for wkey in wildcard_consumer_keys {
            let cons_idxs = consumers.get(&wkey).cloned().unwrap_or_default();
            for (pkey, prod_idxs) in &producers {
                if pkey.0 != wkey.0 {
                    continue;
                }
                if !bucket_matches_wildcard(wkey.1, pkey.1) {
                    continue;
                }
                for &pi in prod_idxs {
                    for &ci in &cons_idxs {
                        pair_one(
                            pi,
                            ci,
                            Some(pkey.1),
                            &mut seen_pairs,
                            &mut named_channel_paired,
                        )?;
                    }
                }
            }
        }

        // Segment-wildcard pass: a Consumer URL containing `{}` segments
        // pairs against any Producer URL whose path matches when those
        // positions are filled by concrete values. Implements the natural
        // route-parameter matching semantics — `/api/trpc/{}` Consumer
        // matches `/api/trpc/polls.list`, `/users/{}/posts` matches
        // `/users/123/posts`, etc.
        //
        // Only runs when the Consumer key contains `{}` AND the Producer key
        // doesn't; the both-sides-already-{} case is handled by the
        // exact-match pass above (template literals normalize to the same
        // `{}` form).
        let segment_wildcard_consumer_keys: Vec<(&str, &str)> = consumers
            .keys()
            .filter(|(_, name)| name.contains("/{}"))
            .copied()
            .collect();
        for wkey in segment_wildcard_consumer_keys {
            let cons_idxs = consumers.get(&wkey).cloned().unwrap_or_default();
            for (pkey, prod_idxs) in &producers {
                if pkey.0 != wkey.0 {
                    continue;
                }
                if pkey.1 == wkey.1 {
                    continue; // Exact-match pass handled this.
                }
                if !segment_wildcard_matches(wkey.1, pkey.1) {
                    continue;
                }
                for &pi in prod_idxs {
                    for &ci in &cons_idxs {
                        pair_one(
                            pi,
                            ci,
                            Some(pkey.1),
                            &mut seen_pairs,
                            &mut named_channel_paired,
                        )?;
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 2: pair DbQuery ↔ DbEntity and MigrationTarget ↔ DbEntity.
    //
    // Entity index keys are the canonical names from DbEntity (both the
    // table_name_hint when present, and the base_name_hint as fallback).
    // Matching is case-insensitive with simple suffix-s pluralization
    // tolerance via `url_pattern::entity_names_match`.
    //
    // A single DbEntity can appear in many pairs — one flow_edge row per
    // (query/migration, entity) combination.
    // -----------------------------------------------------------------------
    let mut db_paired: std::collections::HashSet<usize> = Default::default();

    {
        // Build entity list: (key, idx) pairs.  A DbEntity contributes two
        // entries when both table_name_hint and base_name_hint are non-empty,
        // making it reachable under either name.
        let mut entity_entries: Vec<(&str, usize)> = Vec::new();
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::DbEntity { table_name_hint, base_name_hint, .. } = r.emission {
                if let Some(tname) = table_name_hint.as_deref() {
                    if !tname.is_empty() {
                        entity_entries.push((tname, idx));
                    }
                }
                if !base_name_hint.is_empty() {
                    entity_entries.push((base_name_hint.as_str(), idx));
                }
            }
        }

        let mut pair_stmt = conn
            .prepare_cached(
                "INSERT OR IGNORE INTO flow_edges
                    (source_file_id, source_line, source_symbol,
                     target_file_id, target_line, target_symbol,
                     edge_type, url_pattern, confidence, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .context("Failed to prepare db-paired flow_edges INSERT")?;

        // Helper: find entity indices whose key matches `name` via
        // case-insensitive pluralization-tolerant comparison.
        let find_entities = |name: &str| -> Vec<usize> {
            let mut seen = std::collections::HashSet::new();
            entity_entries.iter()
                .filter(|(key, _)| url_pattern::entity_names_match(name, key))
                .map(|(_, idx)| *idx)
                .filter(|idx| seen.insert(*idx))
                .collect()
        };

        // Pair DbQuery ↔ DbEntity.
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::DbQuery { entity_name, operation } = r.emission {
                if entity_name.is_empty() { continue; }
                for ei in find_entities(entity_name) {
                    let er = &resolved[ei];
                    let n = pair_stmt.execute(rusqlite::params![
                        r.file_id, r.line, qname_for(r.file_id, r.line),
                        er.file_id, er.line, qname_for(er.file_id, er.line),
                        "db_query",
                        Some(entity_name.as_str()),
                        0.85_f64,
                        Some(operation.as_str()),
                    ]).context("Failed to insert db-query flow_edge")?;
                    if n > 0 {
                        db_paired.insert(idx);
                        db_paired.insert(ei);
                    }
                }
            }
        }

        // Pair MigrationTarget ↔ DbEntity.
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::MigrationTarget { table_name, direction } = r.emission {
                if table_name.is_empty() { continue; }
                for ei in find_entities(table_name) {
                    let er = &resolved[ei];
                    let n = pair_stmt.execute(rusqlite::params![
                        r.file_id, r.line, qname_for(r.file_id, r.line),
                        er.file_id, er.line, qname_for(er.file_id, er.line),
                        "migration_target",
                        Some(table_name.as_str()),
                        0.85_f64,
                        Some(direction.as_str()),
                    ]).context("Failed to insert migration-target flow_edge")?;
                    if n > 0 {
                        db_paired.insert(idx);
                        db_paired.insert(ei);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 3: write single-ended rows for everything not already paired.
    // -----------------------------------------------------------------------
    let mut single_stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO flow_edges
                (source_file_id, source_line, source_symbol,
                 edge_type, protocol, http_method, url_pattern, confidence, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .context("Failed to prepare single-ended flow_edges INSERT")?;

    let mut written = named_channel_paired.len() as u32 + db_paired.len() as u32;

    for (idx, r) in resolved.iter().enumerate() {
        if named_channel_paired.contains(&idx) || db_paired.contains(&idx) {
            continue;
        }
        let n = single_stmt.execute(rusqlite::params![
            r.file_id,
            r.line,
            qname_for(r.file_id, r.line),
            r.emission.edge_type(),
            r.emission.protocol(),
            r.emission.http_method_str(),
            r.emission.url_pattern(),
            0.9_f64,
            r.emission.streaming_str(),
        ]).context("Failed to insert single-ended flow_edge")?;
        written += n as u32;
    }

    Ok(written)
}

#[cfg(test)]
pub(crate) fn _test_flush_flow_emissions(
    conn: &rusqlite::Connection,
    emissions: &[(String, u32, FlowEmission)],
) -> Result<u32> {
    flush_flow_emissions(conn, emissions)
}

/// Public entry to the pairing/flush pipeline. Used by post-connector
/// passes (see `full.rs`) that materialise late-arriving DB rows into
/// flow edges. Same semantics as the resolver's own flush.
pub fn flush_flow_emissions_public(
    conn: &rusqlite::Connection,
    emissions: &[(String, u32, FlowEmission)],
) -> Result<u32> {
    flush_flow_emissions(conn, emissions)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

/// Stats returned by `resolve_and_write` / `resolve_iteration`.
#[derive(Debug, Clone, Default)]
pub struct ResolutionStats {
    pub resolved: u64,
    pub engine_resolved: u64,
    pub unresolved: u64,
    pub external: u64,
    /// Chain walker bail-outs collected during this pass. The orchestrator
    /// (full.rs) feeds these into `expand_chain_reachability` to drive a
    /// second-pass `Ecosystem::resolve_symbol` reload.
    pub chain_misses: Vec<ChainMiss>,
}

impl ResolutionStats {
    /// `true` when the chain walker recorded no bail-outs — i.e. no external
    /// file demand was surfaced by this pass and the Stage 2 loop can stop.
    /// Used by the demand-driven pipeline as the fixpoint-exit condition.
    pub fn converged(&self) -> bool {
        self.chain_misses.is_empty()
    }
}

/// Resolve all references across all parsed files, writing edges,
/// unresolved refs, and external refs to the database. One-shot entry
/// point for callers that don't need iteration: runs `resolve_iteration`
/// once and then `finalize_resolution`.
///
/// Two-tier: language-specific resolvers first (1.0 confidence),
/// then heuristic fallback (0.50-0.95 confidence).
/// Unresolvable refs with a known external namespace go to `external_refs`;
/// truly unknown refs go to `unresolved_refs`.
pub fn resolve_and_write(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let stats = resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, false)?;
    finalize_resolution(db)?;
    Ok(stats)
}

/// Incremental variant: augments the SymbolIndex with all symbols from DB
/// so the engine resolver can find targets in unchanged files.
pub fn resolve_and_write_incremental(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let stats = resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, true)?;
    finalize_resolution(db)?;
    Ok(stats)
}

/// One resolution pass without post-processing. Writes edges / external_refs /
/// unresolved_refs the same way as `resolve_and_write` but leaves the
/// `incoming_edge_count` materialization to a later `finalize_resolution`
/// call — so the Stage 2 demand-driven pipeline can call this in a loop,
/// DELETE speculative unresolved/external rows between iterations, and
/// only finalize once the demand set reaches fixpoint.
///
/// `stats.converged()` reports whether the chain walker recorded any
/// bail-outs. Callers use that as the loop-exit signal.
pub fn resolve_iteration(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, false)
}

/// Incremental iteration variant. Same shape as `resolve_iteration` but
/// augments the SymbolIndex with DB symbols first.
pub fn resolve_iteration_incremental(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    resolve_iteration_inner(db, parsed, symbol_id_map, project_ctx, true)
}

/// Reuse-across-iterations variant. The orchestrator (full.rs) builds
/// the SymbolIndex once and threads it through expand-loop iterations
/// via `&mut Option<SymbolIndex>`. Each call:
///   - if `index` is `None`: builds via `build_with_context` (initial)
///   - if `index` is `Some`: reuses, augmenting with `new_files` if non-empty
///
/// On a 280k-symbol aspnetcore index the rebuild costs ~5-10s; running
/// it 8× across the expand loop is ~40-80s of redundant work this
/// avoids. Equivalent correctness for the resolved-edge counts.
pub fn resolve_iteration_with_cached_index(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    cached_index: &mut Option<engine::SymbolIndex>,
    new_files_slice: &[ParsedFile],
) -> Result<ResolutionStats> {
    if cached_index.is_none() {
        let mut index = engine::SymbolIndex::build_with_context(parsed, symbol_id_map, project_ctx);
        let external_paths = read_external_file_paths(db.conn());
        if !external_paths.is_empty() {
            index.set_external_paths(external_paths);
        }
        *cached_index = Some(index);
    } else if !new_files_slice.is_empty() {
        if let Some(idx) = cached_index.as_mut() {
            idx.augment_from_parsed(new_files_slice, symbol_id_map);
        }
    }
    resolve_iteration_inner_with_index(
        db, parsed, symbol_id_map, project_ctx,
        cached_index.as_mut().expect("index is set above"),
    )
}

fn resolve_iteration_inner(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    augment_from_db: bool,
) -> Result<ResolutionStats> {
    // Ext-origin files without an `ext:` path prefix — specifically,
    // script-tag-parsed vendored JS like `wwwroot/lib/jquery.min.js` — live
    // under regular project-relative paths in the DB but carry
    // `origin='external'`. The chain walker's "is this root internal?"
    // filter needs to see them as external so `$`-rooted jQuery chains in
    // user JS classify correctly instead of matching against those vendor
    // symbols as if they were project code.
    let external_paths = read_external_file_paths(db.conn());
    let mut index = SymbolIndex::build_with_context(parsed, symbol_id_map, project_ctx);
    if !external_paths.is_empty() {
        index.set_external_paths(external_paths);
    }

    // For incremental: load symbols from unchanged files so the engine
    // resolver can find cross-file targets (CR #9). The augment SELECT
    // also collects (path, qname) → id pairs so the heuristic gets
    // project-wide coverage from the same scan — eliminates the separate
    // `load_symbol_id_map` full DB scan that incremental.rs used to do.
    let augmented_id_map: Option<HashMap<(String, String), i64>> = if augment_from_db {
        Some(index.augment_from_db_collecting_ids(db.conn()))
    } else {
        None
    };

    resolve_iteration_body(db, parsed, symbol_id_map, project_ctx, &mut index, augmented_id_map)
}

fn resolve_iteration_inner_with_index(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    index: &mut SymbolIndex,
) -> Result<ResolutionStats> {
    resolve_iteration_body(db, parsed, symbol_id_map, project_ctx, index, None)
}

fn resolve_iteration_body(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    index: &mut SymbolIndex,
    augmented_id_map: Option<HashMap<(String, String), i64>>,
) -> Result<ResolutionStats> {
    let engine = ResolutionEngine::new();
    // The closure passed to par_iter requires `&SymbolIndex` (for the
    // SymbolLookup trait), not `&mut SymbolIndex`. Reborrow as
    // immutable for the duration of the loop.
    let index: &SymbolIndex = &*index;

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin resolution transaction")?;

    let mut stats = ResolutionStats::default();

    // Build heuristic lookup structures from the merged symbol map.
    // For full reindex, `symbol_id_map` already covers everything.
    // For incremental, we merge in the `augmented_id_map` so heuristic
    // sees both changed-file IDs (from caller) and unchanged-file IDs
    // (from the augment SELECT) without paying for two full scans.
    let merged_id_map: HashMap<(String, String), i64>;
    let merged_id_map_ref: &HashMap<(String, String), i64> = match augmented_id_map {
        Some(mut m) => {
            m.extend(symbol_id_map.iter().map(|(k, v)| (k.clone(), *v)));
            merged_id_map = m;
            &merged_id_map
        }
        None => symbol_id_map,
    };
    let name_to_ids = heuristic::build_name_index(merged_id_map_ref, parsed);
    let qname_to_id = heuristic::build_qname_index(merged_id_map_ref);
    let module_to_files = heuristic::build_module_to_files(parsed);
    let import_map = heuristic::build_import_map(parsed);
    let file_namespace_map = heuristic::build_file_namespace_map(parsed);

    // Fast companion lookup: HashMap<path, &ParsedFile> replaces the
    // per-iteration O(N) `parsed.iter().find` scan that resolve used
    // before. Bonus: when a companion file ISN'T in `parsed` (incremental
    // run — only the template changed, the component is in the DB), we
    // fall back to pulling that companion's imports directly from the
    // `imports` DB table. Without that fallback, an Angular template
    // edited on its own lost every inherited `.component.ts` import and
    // dropped from ~90% to ~52% resolution rate on touched files.
    let parsed_by_path: std::collections::HashMap<&str, &ParsedFile> =
        parsed.iter().map(|p| (p.path.as_str(), p)).collect();

    // Pre-fetch companion-file imports from DB for files whose companion
    // isn't in this run's parse slice (incremental: companion unchanged,
    // not re-parsed). Keeps the inner per-file loop free of `&Connection`
    // so it can run on rayon workers.
    let companion_db_imports: HashMap<String, Vec<ImportEntry>> = {
        let mut needed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for pf in parsed {
            if pf.path.starts_with("ext:") { continue }
            if let Some(host_resolver) = engine.resolver_for(&pf.language) {
                if let Some(companion_path) = host_resolver.companion_file_for_imports(&pf.path) {
                    if !parsed_by_path.contains_key(companion_path.as_str()) {
                        needed.insert(companion_path);
                    }
                }
            }
        }
        let mut map: HashMap<String, Vec<ImportEntry>> = HashMap::with_capacity(needed.len());
        for path in needed {
            let imports = read_file_imports_from_db(conn, &path);
            if !imports.is_empty() {
                map.insert(path, imports);
            }
        }
        map
    };

    // Per-file resolve runs in parallel via rayon — each worker owns its
    // own `FileWriteBuf` + `FileStats`; results are reduced into the
    // global combined values. The shared inputs (`engine`, `index`,
    // `project_ctx`, all the lookup maps, `parsed_by_path`) are
    // immutable after build so they can be passed by `&` across threads.
    // `SymbolIndex.local_type_cache` is thread-local; chain misses go
    // through a Mutex (bounded contention). The closure captures only
    // `Send + Sync` state.
    //
    // External (`ext:`) files are filtered out — they're indexed for
    // lookup only, never as resolution sources.
    let (mut combined_buf, local_stats_total) = parsed
        .par_iter()
        .filter(|pf| !pf.path.starts_with("ext:"))
        .map(|pf| -> (FileWriteBuf, FileStats) {
            let mut buf = FileWriteBuf::default();
            let mut local_stats = FileStats::default();

            // File-path Consumer pass for mailer templates. A file under a
            // recognised template root (`mails/`, `emails/`, `templates/email/`,
            // `views/mails/`) emits a single Consumer NamedChannel Mailer
            // keyed on the file's basename so a Producer that names the same
            // template pairs against it. Runs once per file at line 1.
            if let Some(template_name) = mailer_template_name_for_path(&pf.path) {
                buf.flow_emissions.push((
                    pf.path.clone(),
                    1u32,
                    flow_emit::FlowEmission::NamedChannel {
                        kind: flow_emit::NamedChannelKind::Mailer,
                        name: template_name,
                        role: flow_emit::ChannelRole::Consumer,
                        method: None,
                    streaming: None,
                    },
                ));
            }

            // File-path Consumer pass for Next.js routes. App Router files
            // matching `**/app/**/route.{ts,tsx,js,jsx}` emit one Consumer
            // per exported HTTP-verb handler (`GET`/`POST`/…). Pages Router
            // files matching `**/pages/api/**/*.{ts,…}` emit a single Any-
            // method Consumer keyed on the file's URL path.
            for emission in nextjs_route_consumer_emissions(&pf.path, &pf.symbols) {
                buf.flow_emissions.push((pf.path.clone(), 1u32, emission));
            }

            // Cross-language adapter: every `ExtractedRoute` populated by a
            // language extractor (ASP.NET attribute routes, Phoenix routes,
            // chi/gin/echo routes, Spring `@GetMapping`, etc.) becomes a
            // Consumer NamedChannel HttpCall so it can pair against
            // Producer-side HTTP calls from any other language.
            for (line, emission) in extracted_routes_to_emissions(&pf.routes, &pf.symbols) {
                buf.flow_emissions.push((pf.path.clone(), line, emission));
            }

            // Extractor-time `FlowEmission`s — populated at parse time by
            // file-structure scanners that can't migrate to chain-walk
            // resolver-time emission (GraphQL SDL parsing, `.proto` service
            // blocks).
            plugin_flow_emissions_to_emissions(pf, &mut buf.flow_emissions);

            // ExtractedDbSet → DbEntity adapter. Languages with model-table
            // extraction (currently C# EF Core via `DbSet<T>` properties on
            // DbContext classes) emit one ExtractedDbSet per discovered
            // model. Each becomes a `FlowEmission::DbEntity` so DbQuery
            // emissions for the same entity name pair against it via the
            // pairer's entity matcher.
            for (line, emission) in extracted_db_sets_to_emissions(&pf.db_sets, &pf.symbols) {
                buf.flow_emissions.push((pf.path.clone(), line, emission));
            }

        // Look up source IDs against the MERGED map so incremental
        // resolves can find symbol IDs for files that weren't in this
        // run's `parsed` slice (e.g. blast-radius files re-parsed for
        // resolve but whose IDs come from the augment SELECT, not from
        // the caller's changed-only map).
        let file_symbol_ids: Vec<Option<i64>> = pf
            .symbols
            .iter()
            .map(|sym| {
                merged_id_map_ref
                    .get(&(pf.path.clone(), sym.qualified_name.clone()))
                    .copied()
            })
            .collect();

        // Try language-specific resolver for this file.
        let host_resolver = engine.resolver_for(&pf.language);
        let host_file_ctx = host_resolver.map(|r| {
            let mut ctx = r.build_file_context(pf, project_ctx);
            // Merge companion imports (e.g. Angular template inherits the
            // paired `.component.ts` imports, since the template itself has
            // no import statements but every symbol it names is imported by
            // the component class).
            if let Some(companion_path) = r.companion_file_for_imports(&pf.path) {
                if let Some(comp_pf) = parsed_by_path.get(companion_path.as_str()) {
                    if let Some(comp_resolver) = engine.resolver_for(&comp_pf.language) {
                        let comp_ctx = comp_resolver.build_file_context(comp_pf, project_ctx);
                        ctx.imports.extend(comp_ctx.imports);
                    }
                } else if let Some(db_imports) = companion_db_imports.get(&companion_path) {
                    // Companion file isn't in this run's parse slice
                    // (incremental re-index, companion unchanged). The
                    // imports were prefetched into `companion_db_imports`
                    // before the parallel section so the per-file body
                    // doesn't need `conn`.
                    ctx.imports.extend(db_imports.iter().cloned());
                }
            }
            ctx
        });

        let empty_vec = vec![];
        let file_imports = import_map.get(&pf.path).unwrap_or(&empty_vec);
        let source_namespace = file_namespace_map.get(&pf.path).map(|s| s.as_str());

        // R5: install a fresh per-file flow-typing cache. Narrowings are
        // sorted innermost-first (smallest range first) so the cache's
        // cursor-based lookup picks the most specific scope on ties.
        let mut narrowings = pf.flow.narrowings.clone();
        narrowings.sort_by_key(|n| n.byte_end.saturating_sub(n.byte_start));
        index.install_local_cache(narrowings);

        // R5: iterate refs in source order so forward inference
        // (`let x = foo(); x.bar()`) propagates correctly. Reassignment is
        // handled naturally by last-write-wins in the cache. We keep the
        // original ref_idx alongside so flow_binding_lhs / origin-language
        // lookups stay correct after the sort.
        //
        // Only sort when the file actually uses flow-typing — the sort
        // perturbs extractor emission order, which INSERT OR IGNORE on
        // edges is sensitive to for duplicate-target refs. When no flow
        // metadata is present, preserve the original order.
        let uses_flow = !pf.flow.flow_binding_lhs.is_empty()
            || !pf.flow.narrowings.is_empty();
        let refs_ordered: Vec<(usize, &crate::types::ExtractedRef)> = if uses_flow {
            let mut v: Vec<_> = pf.refs.iter().enumerate().collect();
            v.sort_by_key(|(_, r)| r.line);
            v
        } else {
            pf.refs.iter().enumerate().collect()
        };

        for (ref_idx, r) in refs_ordered {
            // Determine the effective language for this ref. Refs from embedded
            // regions (e.g. TS inside a Vue/Svelte file, JS inside PHP/Elixir)
            // carry their own language tag so the resolver and
            // externals/primitives classification use the correct language's
            // ruleset rather than the host language's.
            let effective_lang: &str = pf
                .ref_origin_languages
                .get(ref_idx)
                .and_then(|o| o.as_deref())
                .unwrap_or(&pf.language);
            // Whether this ref belongs to a different language than the host.
            let is_cross_lang_embedded = effective_lang != pf.language.as_str();

            // For cross-language embedded refs, look up the resolver for the
            // embedded language. For same-language refs, reuse the host resolver
            // and file_ctx already computed for this file.
            //
            // Example: a `.vue` file (host = "vue") has `<script lang="ts">`.
            // Embedded TS refs get effective_lang = "typescript" → we use the
            // TypeScript resolver and build a fresh file_ctx from the same
            // ParsedFile (which contains all embedded symbols/imports merged in).
            let (resolver, file_ctx): (Option<&dyn engine::LanguageResolver>, _) =
                if is_cross_lang_embedded {
                    let emb_resolver = engine.resolver_for(effective_lang);
                    let emb_ctx = emb_resolver.map(|res| res.build_file_context(pf, project_ctx));
                    (emb_resolver, emb_ctx)
                } else {
                    (host_resolver, host_file_ctx.clone())
                };

            let source_id = match file_symbol_ids.get(r.source_symbol_index).and_then(|id| *id) {
                Some(id) => id,
                None => continue,
            };

            // Wildcard imports (`use foo::*`) are scope-declaration statements, not
            // missing-symbol references.  They cannot resolve to a single target and
            // should not appear in the unresolved_refs table.
            if r.kind == EdgeKind::Imports && r.target_name == "*" {
                local_stats.external += 1; // count as "handled" so they don't inflate unresolved rate
                continue;
            }

            // R5: move the flow-cache cursor to this ref's byte offset so
            // narrowing lookups in chain walkers see the right scope.
            // Sprint 1 leaves this as 0 for languages that haven't wired
            // their FlowConfig yet — narrowings are empty in that case so
            // the cursor value doesn't matter.
            let ref_byte = pf
                .flow
                .ref_byte_offsets
                .get(ref_idx)
                .copied()
                .unwrap_or(0);
            index.set_cursor(ref_byte);

            // Tier 1: Try language-specific resolver (for the effective language).
            let mut resolved_by_engine = false;
            if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let source_sym = &pf.symbols[r.source_symbol_index];
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
                    file_package_id: pf.package_id,
                };

                // Flow-emission detection runs regardless of whether resolution
                // succeeds — HTTP client calls, IPC, WebSocket emits, etc. are
                // identifiable from import context alone, even when the chain
                // walker can't resolve the external symbol to a DB id.
                for emission in resolver.detect_flow_emission_with_lookup(file_ctx, &ref_ctx, index) {
                    buf.flow_emissions.push((pf.path.clone(), r.line, emission));
                }

                if let Some(resolution) = resolver.resolve(file_ctx, &ref_ctx, index) {
                    // R5: if this ref is the RHS of `<lhs> = <expr>`, record
                    // the target's yield type (return type for a method call,
                    // declared type for a field) against the named LHS. The
                    // chain walker already populates `resolution.resolved_yield_type`
                    // for chain refs; fall back to looking up the target's
                    // return/field type directly so single-segment call refs
                    // like `let x = foo()` also drive forward inference.
                    if let Some(lhs_idx) = pf.flow.flow_binding_lhs.get(&ref_idx).copied() {
                        let yield_type = resolution.resolved_yield_type.clone().or_else(|| {
                            let target_id = resolution.target_symbol_id;
                            // Recover the target's qualified name from the index
                            // and look up its return/field type.
                            index
                                .by_name(&r.target_name)
                                .iter()
                                .find(|s| s.id == target_id)
                                .and_then(|s| {
                                    index
                                        .return_type_name(&s.qualified_name)
                                        .or_else(|| {
                                            index.field_type_name(&s.qualified_name)
                                        })
                                        .map(|t| t.to_string())
                                })
                        });
                        if let Some(yield_type) = yield_type {
                            if let Some(lhs_sym) = pf.symbols.get(lhs_idx) {
                                index.record_local_type(
                                    lhs_sym.name.clone(),
                                    yield_type,
                                );
                            }
                        }
                    }

                    buf.edges.push((
                        source_id,
                        resolution.target_symbol_id,
                        r.kind.as_str(),
                        r.line,
                        resolution.confidence,
                        resolution.strategy,
                    ));
                    // When both detect_flow_emission and resolution.flow_emit are
                    // set, prefer the resolution-attached one (it may carry richer
                    // data from the chain walker). The detect_flow_emission path
                    // already pushed its entry above; avoid double-emitting.
                    if let Some(emission) = resolution.flow_emit {
                        buf.flow_emissions.push((pf.path.clone(), r.line, emission));
                    }
                    local_stats.resolved += 1;
                    local_stats.engine_resolved += 1;
                    resolved_by_engine = true;
                }
            }

            if resolved_by_engine {
                continue;
            }

            // Build scope context once for remaining classification steps.
            let source_sym = &pf.symbols[r.source_symbol_index];
            let scope_chain = build_scope_chain(source_sym.scope_path.as_deref());

            // ---------------------------------------------------------------
            // Tier 1.1: Generic type parameter resolution.
            // If this is a TypeRef and the target matches a generic param
            // declared on an enclosing type (e.g., `T` in `class Repo<T>`),
            // it's a type parameter — not a missing symbol.
            // ---------------------------------------------------------------
            if r.kind == EdgeKind::TypeRef {
                // Walk the source symbol's own qualified name first, then up
                // through its parents. The function/struct itself owns its
                // type parameters; refs in its signature have its qname (not
                // its parent) as the relevant scope for generic-param lookup.
                let is_generic_param = std::iter::once(source_sym.qualified_name.as_str())
                    .chain(scope_chain.iter().map(String::as_str))
                    .any(|scope| {
                        index
                            .generic_params(scope)
                            .map_or(false, |params| params.iter().any(|p| p == &r.target_name))
                    });
                if is_generic_param {
                    buf.externals.push((
                        source_id,
                        r.target_name.clone(),
                        r.kind.as_str(),
                        r.line,
                        "generic_param".to_string(),
                        pf.package_id,
                    ));
                    local_stats.external += 1;
                    continue;
                }
            }

            // ---------------------------------------------------------------
            // Tier 1.5: External classification — BEFORE heuristic.
            //
            // Language resolvers and chain inference can identify refs that
            // belong to external packages (stdlib, third-party crates).
            // Check this FIRST so the heuristic doesn't create false
            // low-confidence edges for things like `map`, `iter`, `get`
            // that match internal method names by coincidence.
            //
            // `resolver` and `file_ctx` here are already the effective-language
            // versions (embedded resolver for cross-lang refs, host resolver
            // otherwise), so no special-casing is needed.
            // ---------------------------------------------------------------
            let inferred_ns = if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: scope_chain.clone(),
                    file_package_id: pf.package_id,
                };
                resolver.infer_external_namespace_with_lookup(
                    file_ctx, &ref_ctx, project_ctx, index,
                )
            } else {
                None
            };

            // Chain-to-external: if the chain walks to a type not in the index,
            // classify as external (handles ORM, test framework, fluent API chains).
            let inferred_ns = inferred_ns.or_else(|| {
                r.chain.as_ref().and_then(|chain| {
                    engine::infer_external_from_chain(chain, &scope_chain, index)
                })
            });

            // Bare-name external check: test globals, language primitives,
            // and runtime builtins — classified with specific namespaces.
            // Use effective_lang so JS/TS refs embedded in Elixir/PHP/Ruby
            // host files are classified against the JS/TS primitive/builtin
            // tables, not the host language's.
            let inferred_ns = inferred_ns.or_else(|| {
                index
                    .classify_external_name(&r.target_name, effective_lang)
                    .map(|ns| ns.to_string())
            });

            // Import-based external for bare usages: if the ref's name (or its
            // leading segment, for qualified targets like `Stripe.Event`) matches
            // an entry in this file's import list whose source module has zero
            // local symbols in the project index, classify as external.
            //
            // Catches three practical cases the language-specific manifest
            // checks miss:
            //   1. Transitive bare-package deps — e.g. Java `import
            //      tools.jackson.databind.ObjectMapper` (Jackson 3.x, pulled in
            //      via spring-boot-starter, not declared in pom.xml) or Python
            //      `from sqlalchemy import Engine` (transitive of sqlmodel).
            //   2. Bare-package deep imports like `rxjs/operators`,
            //      `lodash/fp`, `date-fns/utcToZonedTime` — the slash-bearing
            //      specifier isn't a relative path, it's a sub-module of an
            //      indexed package that the manifest may not enumerate.
            //   3. Relative imports to files that don't exist in the index —
            //      e.g. NSwag-generated `'../web-api-client'` that's produced
            //      at build time and absent at scan time.
            //
            // Language-agnostic: relies only on the project's own symbol
            // index via `is_module_in_project` as the sole "is this actually
            // local?" authority. Imports whose target the module resolvers
            // couldn't reach, by any path, are called external — honest to
            // "we don't have its definition" without inventing a fake edge.
            let inferred_ns = inferred_ns.or_else(|| {
                if r.module.is_some() {
                    return None;
                }
                let target = r.target_name.as_str();
                let first_segment = target.split('.').next().unwrap_or(target);
                for (imported_name, module_path_opt) in file_imports.iter() {
                    if imported_name != target && imported_name != first_segment {
                        continue;
                    }
                    let Some(module_path) = module_path_opt.as_deref() else {
                        continue;
                    };
                    if is_module_in_project(module_path, &module_to_files, index) {
                        continue;
                    }
                    return Some(format!("ext:{module_path}"));
                }
                None
            });

            // Module-qualified external check: if the ref has module="X" and
            // "X" is not a local file/namespace, classify as external.
            // e.g., R `dplyr::mutate` → dplyr not local → external.
            //       Erlang `lists:map` → lists not local → external.
            //       Haskell `Map.lookup` → Map not local → external.
            let inferred_ns = inferred_ns.or_else(|| {
                if let Some(module) = &r.module {
                    let mod_lower = module.to_lowercase();
                    // Full-path match: "Ecto.Changeset" or "dplyr" as-is.
                    let mut is_local = module_to_files.contains_key(module.as_str())
                        || module_to_files.contains_key(&mod_lower);

                    // Last-segment match ONLY for single-segment modules.
                    // Multi-segment modules like "Ecto.Changeset" should NOT
                    // be classified as local just because a file named
                    // "changeset.ex" exists — that's a coincidental stem match.
                    if !is_local && !module.contains('.') {
                        let last_seg = module.rsplit('.').next().unwrap_or(module);
                        let last_lower = last_seg.to_lowercase();
                        is_local = module_to_files.contains_key(last_seg)
                            || module_to_files.contains_key(&last_lower);
                    }

                    if !is_local {
                        Some(format!("ext:{module}"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            if let Some(ns) = &inferred_ns {
                buf.externals.push((
                    source_id,
                    r.target_name.clone(),
                    r.kind.as_str(),
                    r.line,
                    ns.clone(),
                    pf.package_id,
                ));
                local_stats.external += 1;
                continue;
            }

            // Tier 2: Heuristic fallback.
            let ref_module = r.module.as_deref();
            let chain_prefix = r.chain.as_ref().and_then(|c| {
                if c.segments.len() >= 2 {
                    Some(c.segments[c.segments.len() - 2].name.as_str())
                } else {
                    None
                }
            });
            let resolution = heuristic::resolve_ref(
                r.target_name.as_str(),
                r.kind,
                &pf.path,
                file_imports,
                source_namespace,
                chain_prefix,
                ref_module,
                &name_to_ids,
                &qname_to_id,
                &module_to_files,
                symbol_id_map,
                parsed,
                &|p| index.is_ambient_path(p),
                &|suffix, prefix, module, cands| {
                    index.resolve_via_external_reexport(suffix, prefix, module, cands)
                },
            );

            match resolution {
                Some((target_id, confidence, strategy)) => {
                    buf.edges.push((
                        source_id,
                        target_id,
                        r.kind.as_str(),
                        r.line,
                        confidence,
                        strategy,
                    ));
                    local_stats.resolved += 1;
                }
                None => {
                    // Truly unresolved — no external namespace identified,
                    // no heuristic match found.
                    //
                    // Guard: the outer loop skips ext: files entirely, but a
                    // symbol's file_path could still be external (e.g. augmented
                    // from DB during incremental). Don't pollute unresolved_refs
                    // with gaps from third-party code — only project code's
                    // unresolved refs are the user's concern.
                    if pf.path.starts_with("ext:") {
                        continue;
                    }
                    // Imports edges point at a module, not a symbol. The
                    // heuristic can't bind them because the module name is
                    // a file stem rather than an identifier. Classify
                    // import edges generically:
                    //
                    //   * If the module name resolves to a project file
                    //     stem via `module_to_files`, the import is
                    //     satisfied locally — count as handled.
                    //   * If the leaf name appears in the SymbolIndex
                    //     under an `ext:` path, the import points at an
                    //     indexed external surface — count as handled.
                    //   * Otherwise the import points at a third-party
                    //     dependency the package manager didn't surface
                    //     (Nimble package not installed, Cabal package
                    //     not in the store). The dep is external by
                    //     definition; we just don't have its source.
                    //     Classify as external rather than unresolved so
                    //     "couldn't find symbol" stays distinct from
                    //     "import points at uninstalled dep".
                    if r.kind == EdgeKind::Imports {
                        let probe = r
                            .module
                            .as_deref()
                            .filter(|m| !m.is_empty())
                            .unwrap_or(r.target_name.as_str());
                        if is_module_in_project(probe, &module_to_files, index) {
                            local_stats.external += 1;
                            continue;
                        }
                        let leaf = probe.rsplit(['/', '.', ':']).next().unwrap_or(probe);
                        if !leaf.is_empty() {
                            let any_external = index
                                .by_name(leaf)
                                .iter()
                                .any(|s| s.file_path.starts_with("ext:"));
                            if any_external {
                                local_stats.external += 1;
                                continue;
                            }
                        }
                        // Uninstalled / unwalked third-party dep. The
                        // import is real, the source just isn't on disk.
                        local_stats.external += 1;
                        continue;
                    }
                    let module_value = r.module.as_deref().map(|s| s.to_string());
                    // E3: propagate snippet flag from source symbol for
                    // aggregate-stats exclusion.
                    let from_snippet = pf
                        .symbol_from_snippet
                        .get(r.source_symbol_index)
                        .copied()
                        .unwrap_or(false);
                    buf.unresolved.push((
                        source_id,
                        r.target_name.clone(),
                        r.kind.as_str(),
                        r.line,
                        module_value,
                        pf.package_id,
                        from_snippet,
                    ));
                    local_stats.unresolved += 1;
                }
            }
        }

            // R5: wipe the local-type cache so bindings from this file don't
            // leak into the next file processed on the same rayon worker.
            // (TLS cache survives across rayon tasks on the same worker
            // thread; explicit clear keeps it tight.)
            index.clear_local_cache();

            (buf, local_stats)
        })
        .reduce(
            || (FileWriteBuf::default(), FileStats::default()),
            |(mut buf_a, mut stats_a), (buf_b, stats_b)| {
                buf_a.merge(buf_b);
                stats_a.merge(stats_b);
                (buf_a, stats_a)
            },
        );

    // Bulk-flush the per-file buffers in one transaction. Multi-row
    // VALUES inserts cut driver round-trips ~Nx vs the previous per-ref
    // path. Identical chunk sizes hit the rusqlite stmt cache.
    flush_resolve_buf(&tx, &combined_buf)?;
    stats.resolved += local_stats_total.resolved;
    stats.engine_resolved += local_stats_total.engine_resolved;
    stats.unresolved += local_stats_total.unresolved;
    stats.external += local_stats_total.external;

    tx.commit()
        .context("Failed to commit resolution transaction")?;

    // Flush resolver-emitted flow edges. Runs after the main transaction
    // so it can use the committed file rows for the file_id lookup.
    if !combined_buf.flow_emissions.is_empty() {
        let n = flush_flow_emissions(conn, &combined_buf.flow_emissions)?;
        if n > 0 {
            info!("Resolver-emitted flow edges: {n}");
        }
    }

    // Drain chain walker bail-outs for the orchestrator's R3 reload pass.
    // Deduped on (current_type, target_name) — the second pass cares about
    // unique misses; the source-ref retry list is recovered from the DB's
    // `unresolved_refs` table.
    let raw_misses = index.take_chain_misses();
    let mut seen: std::collections::HashSet<ChainMiss> = std::collections::HashSet::new();
    let mut unique_misses: Vec<ChainMiss> = Vec::new();
    for m in raw_misses.iter().cloned() {
        if seen.insert(m.clone()) { unique_misses.push(m); }
    }
    if !unique_misses.is_empty() {
        debug!(
            "Chain walker recorded {} bail-outs ({} unique)",
            raw_misses.len(),
            unique_misses.len(),
        );
    }
    stats.chain_misses = unique_misses;

    if stats.engine_resolved > 0 || stats.external > 0 {
        info!(
            "Resolution: {} by engine, {} by heuristic, {} external, {} unresolved",
            stats.engine_resolved,
            stats.resolved - stats.engine_resolved,
            stats.external,
            stats.unresolved,
        );
    }

    Ok(stats)
}

/// Post-resolution DB maintenance. Runs once after the last call to
/// `resolve_iteration` in a demand-driven loop (or immediately after the
/// one-shot `resolve_and_write`). Rematerializes `incoming_edge_count` on
/// every symbol row so centrality / blast-radius queries stay O(1).
///
/// The join-update path (scan edges into a temp table, UPDATE once) is
/// O(E + S log D) — far cheaper than a correlated subquery that would
/// issue one COUNT per symbol row. Separated from `resolve_iteration` so
/// Stage 2 can call iteration multiple times without paying this cost on
/// every pass.
pub fn finalize_resolution(db: &mut Database) -> Result<()> {
    {
        let conn = db.conn();
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _edge_counts (id INTEGER PRIMARY KEY, cnt INTEGER);
             DELETE FROM _edge_counts;
             INSERT INTO _edge_counts SELECT target_id, COUNT(*) FROM edges GROUP BY target_id;",
        )
        .context("Failed to build edge count temp table")?;
        conn.execute(
            "UPDATE symbols SET incoming_edge_count = COALESCE(
                (SELECT cnt FROM _edge_counts WHERE _edge_counts.id = symbols.id), 0)",
            [],
        )
        .context("Failed to materialize incoming_edge_count")?;
        conn.execute("DELETE FROM _edge_counts", [])
            .context("Failed to clean up edge count temp table")?;
    }

    // L2+L1+L3 of the reachability-based dead-code stack:
    //   1. Synthesize dispatch_candidate edges so virtual-dispatch
    //      targets (trait/interface method → impl method) are present
    //      in the graph before the BFS runs.
    //   2. Rebuild the `entry_points` table from contributors.
    //   3. BFS from entry-point seeds through edges above the confidence
    //      threshold to materialize `reachability`.
    // Dead-code queries antijoin against `reachability` instead of
    // checking `incoming_edge_count = 0`.
    synthesize_dispatch::synthesize_dispatch_edges(db)
        .context("Failed to synthesize dispatch edges")?;
    crate::query::entry_points::rebuild_entry_points(db)
        .context("Failed to rebuild entry_points")?;
    reachability::materialize_reachability(db)
        .context("Failed to materialize reachability")?;
    crate::query::dead_code::materialize_package_resolution_health(db)
        .context("Failed to materialize package_resolution_health")?;

    Ok(())
}

/// Does the project's symbol index cover this import module?
///
/// Returns true if the module appears as a local namespace (any symbol has
/// that prefix) or maps to a local file via the heuristic module-to-file map.
/// A module that walks through multiple segments (`a.b.c`) is local when any
/// of those segments is covered — this prevents a false "external" classification
/// for package-qualified imports like Python `from app.core.db import engine`.
///
/// Relative specifiers (`./foo`, `../bar/Baz.astro`) are probed by the
/// trailing basename stem — `module_to_files` is keyed by stem, not by
/// fully-qualified path. Without this, every relative-path import of an
/// indexed file would appear external just because the map can't be queried
/// with the raw `../` form.
fn is_module_in_project(
    module_path: &str,
    module_to_files: &rustc_hash::FxHashMap<String, Vec<String>>,
    index: &engine::SymbolIndex,
) -> bool {
    if module_to_files.contains_key(module_path) {
        return true;
    }
    let lower = module_path.to_lowercase();
    if lower != module_path && module_to_files.contains_key(&lower) {
        return true;
    }
    if index.has_in_namespace(module_path) {
        return true;
    }
    // Relative/slash-bearing path: probe by trailing basename.
    // `../../components/Aside.astro` → basename `Aside.astro` → stem `Aside`.
    // `./auth.service`               → basename `auth.service` (extension-less;
    //                                   `build_module_to_files` stores `auth.service`).
    // Try the full basename as-is (for extension-less module paths, where the
    // file `auth.service.ts` is keyed as `auth.service` in module_to_files)
    // AND the once-stripped stem (for paths carrying an explicit extension
    // like `Aside.astro`). Whichever matches first wins.
    if let Some(basename) = module_path.rsplit(['/', '\\']).next() {
        if basename != module_path && !basename.is_empty() {
            if module_to_files.contains_key(basename) {
                return true;
            }
            let basename_lower = basename.to_lowercase();
            if basename_lower != basename
                && module_to_files.contains_key(&basename_lower)
            {
                return true;
            }
            if let Some((stem, _)) = basename.rsplit_once('.') {
                if !stem.is_empty() {
                    if module_to_files.contains_key(stem) {
                        return true;
                    }
                    let stem_lower = stem.to_lowercase();
                    if stem_lower != stem
                        && module_to_files.contains_key(&stem_lower)
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Pull the set of `origin='external'` file paths from the `files` table.
///
/// Used to seed the SymbolIndex so chain walkers can tell
/// script-tag-parsed vendor JS (`wwwroot/lib/jquery.min.js`, written with
/// `origin='external'` but keeping its regular project-relative path)
/// apart from user source. Without this signal, the internal filter in
/// `infer_external_from_chain` — which only checks for an `ext:` path
/// prefix — would count a vendored `$` / `jQuery` declaration as project
/// code and suppress external classification for every jQuery chain in
/// the user's own JS.
///
/// Returns an empty set on any SQL error; a missing external_paths set
/// degrades gracefully to the old `ext:`-prefix-only behaviour.
fn read_external_file_paths(
    conn: &rusqlite::Connection,
) -> std::collections::HashSet<String> {
    let sql = "SELECT path FROM files WHERE origin = 'external'";
    let Ok(mut stmt) = conn.prepare_cached(sql) else {
        return std::collections::HashSet::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return std::collections::HashSet::new();
    };
    rows.filter_map(|r| r.ok()).collect()
}

/// Pull a file's persisted `imports` rows directly from the DB and
/// rehydrate them as `ImportEntry` values. Used by the companion-import
/// merge when the companion file isn't in the current parse slice
/// (incremental re-index where only the template changed but the paired
/// component was already indexed in a prior run).
///
/// Returns an empty Vec on any SQL error; the path through here is best-
/// effort context enrichment, not a correctness-critical read.
/// If `path` lives under a recognised mailer template root, return the
/// file's basename (without extension) as the template name. Returns `None`
/// for any other path. The supported roots match the conventions of
/// Nodemailer Express handlebars (`emails/`), NestJS mailer module
/// (`mails/`, `templates/email/`), and Rails-style mailers (`views/mails/`,
/// `app/views/mailers/`). The match is path-segment exact — a file named
/// `MyEmail.tsx` ten levels deep under any non-template ancestor is not
/// matched.
fn mailer_template_name_for_path(path: &str) -> Option<String> {
    const ROOTS: &[&[&str]] = &[
        &["mails"],
        &["emails"],
        &["templates", "email"],
        &["templates", "emails"],
        &["templates", "mail"],
        &["templates", "mails"],
        &["views", "mails"],
        &["views", "mailers"],
        &["app", "views", "mailers"],
    ];
    let norm: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if norm.len() < 2 {
        return None;
    }
    let basename = norm.last()?;
    let stem = basename.rsplit_once('.').map(|(s, _)| s).unwrap_or(basename);
    if stem.is_empty() {
        return None;
    }
    let parent_segments = &norm[..norm.len() - 1];
    for root in ROOTS {
        if parent_segments
            .windows(root.len())
            .any(|w| w.iter().zip(root.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b)))
        {
            return Some(stem.to_string());
        }
    }
    None
}

/// Recognise Next.js HTTP route files and return Consumer NamedChannel
/// HttpCall emissions for them. Two route conventions are supported:
///
/// - **App Router** (`**/app/**/route.{ts,tsx,js,jsx}`): one Consumer per
///   exported HTTP-verb function (`GET` / `POST` / `PUT` / `PATCH` /
///   `DELETE` / `HEAD` / `OPTIONS`). The URL pattern is the file's path with
///   the `app/` ancestor stripped, the `route.<ext>` basename dropped, and
///   `[id]` / `[...slug]` / `[[...slug]]` segments rewritten to `{}`.
/// - **Pages Router** (`**/pages/api/**/*.{ts,…}`): a single Any-method
///   Consumer per file (Pages Router handlers dispatch internally on
///   `req.method`). URL pattern excludes the `.ts` extension and rewrites
///   `[id]` segments the same way; the special `index` basename is
///   collapsed to the directory.
///
/// Returns an empty Vec when the path doesn't match either convention or
/// when the App Router file exports no recognised HTTP-verb handlers.
fn nextjs_route_consumer_emissions(
    path: &str,
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<flow_emit::FlowEmission> {
    let segs: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Vec::new();
    }
    let basename = *segs.last().unwrap();

    // App Router: `app/**/route.{ts,tsx,js,jsx}` — exports name the verbs.
    if matches!(basename, "route.ts" | "route.tsx" | "route.js" | "route.jsx") {
        let Some(app_idx) = segs.iter().rposition(|s| *s == "app") else {
            return Vec::new();
        };
        // Drop the `route.<ext>` basename and rewrite dynamic segments.
        let url_segs: Vec<String> = segs[app_idx + 1..segs.len() - 1]
            .iter()
            .map(|s| rewrite_nextjs_segment(s))
            .collect();
        let url = if url_segs.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", url_segs.join("/"))
        };
        let verbs = exported_http_verb_handlers(symbols);
        if !verbs.is_empty() {
            return verbs
                .into_iter()
                .map(|method| flow_emit::FlowEmission::NamedChannel {
                    kind: flow_emit::NamedChannelKind::HttpCall,
                    name: url.clone(),
                    role: flow_emit::ChannelRole::Consumer,
                    method: Some(method),
                streaming: None,
                })
                .collect();
        }
        // Fallback: Next.js App Router requires route files to export at
        // least one HTTP verb. Re-export-rename patterns
        // (`export { handler as GET, handler as POST }`) currently aren't
        // synthesised as named symbols when the original is a local
        // declaration. Emit a single Any-method Consumer so the file still
        // pairs against any-method Producers (incl. tRPC client emissions).
        return vec![flow_emit::FlowEmission::NamedChannel {
            kind: flow_emit::NamedChannelKind::HttpCall,
            name: url,
            role: flow_emit::ChannelRole::Consumer,
            method: Some(flow_emit::HttpMethod::Any),
        streaming: None,
        }];
    }

    // Pages Router: `pages/api/**` with any TS/JS extension. Skip files
    // whose basename starts with `_` (Next.js convention for non-route).
    let pages_idx = segs.iter().rposition(|s| *s == "pages");
    let api_after_pages = pages_idx.and_then(|p_idx| {
        segs.get(p_idx + 1).filter(|s| **s == "api").map(|_| p_idx + 1)
    });
    if let Some(api_idx) = api_after_pages {
        if basename.starts_with('_') {
            return Vec::new();
        }
        let stem = basename
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(basename);
        if !matches!(
            basename.rsplit_once('.').map(|(_, ext)| ext).unwrap_or(""),
            "ts" | "tsx" | "js" | "jsx" | "mjs"
        ) {
            return Vec::new();
        }
        let mut url_segs: Vec<String> = segs[api_idx..segs.len() - 1]
            .iter()
            .map(|s| rewrite_nextjs_segment(s))
            .collect();
        // Pages convention: `pages/api/users/index.ts` → `/api/users`.
        if stem != "index" {
            url_segs.push(rewrite_nextjs_segment(stem));
        }
        let url = format!("/{}", url_segs.join("/"));
        return vec![flow_emit::FlowEmission::NamedChannel {
            kind: flow_emit::NamedChannelKind::HttpCall,
            name: url,
            role: flow_emit::ChannelRole::Consumer,
            method: Some(flow_emit::HttpMethod::Any),
        streaming: None,
        }];
    }

    Vec::new()
}

/// Rewrite a single Next.js route segment for URL normalization:
/// - `[id]` → `{}`           (single dynamic segment)
/// - `[...slug]` → `{}`      (catch-all)
/// - `[[...slug]]` → `{}`    (optional catch-all)
/// - `(group)` → ``          (route group; consumed by the join, see below)
/// - literal segments returned unchanged.
///
/// Route groups (`(marketing)`) are returned as empty strings; the caller is
/// expected to filter empties out of the joined URL.
fn rewrite_nextjs_segment(seg: &str) -> String {
    if seg.starts_with('(') && seg.ends_with(')') {
        return String::new();
    }
    if seg.starts_with("[[...") && seg.ends_with("]]") {
        return "{}".to_string();
    }
    if seg.starts_with("[...") && seg.ends_with(']') {
        return "{}".to_string();
    }
    if seg.starts_with('[') && seg.ends_with(']') {
        return "{}".to_string();
    }
    seg.to_string()
}

/// Return the set of HTTP methods exported as named functions from a file.
/// Recognises the canonical Next.js App Router handler names (uppercase
/// HTTP verbs). Order is fixed (Get, Post, Put, Patch, Delete, Head, Options)
/// for deterministic emission across runs.
fn exported_http_verb_handlers(
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<flow_emit::HttpMethod> {
    use flow_emit::HttpMethod;
    let mut found = [false; 7];
    for sym in symbols {
        match sym.name.as_str() {
            "GET" => found[0] = true,
            "POST" => found[1] = true,
            "PUT" => found[2] = true,
            "PATCH" => found[3] = true,
            "DELETE" => found[4] = true,
            "HEAD" => found[5] = true,
            "OPTIONS" => found[6] = true,
            _ => {}
        }
    }
    let order = [
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Patch,
        HttpMethod::Delete,
        HttpMethod::Head,
        HttpMethod::Options,
    ];
    found
        .iter()
        .zip(order.iter())
        .filter_map(|(f, m)| if *f { Some(*m) } else { None })
        .collect()
}

/// Convert per-file `ExtractedRoute` entries into Consumer
/// `NamedChannel { kind: HttpCall, .. }` emissions. Used by the cross-language
/// adapter to surface HTTP routes from any language that participates in
/// `pf.routes` (C# ASP.NET attribute routes, Elixir Phoenix scope blocks,
/// Go chi/gin chains, Java Spring `@GetMapping`, …) — they pair against
/// Producer-side HTTP calls keyed on the same URL.
///
/// Returns `(handler_line, emission)` pairs. Routes whose `template` is
/// empty after trimming are skipped — without a URL there's no pairing
/// key. The `http_method` field is parsed via `HttpMethod::from_method_name`
/// so `"GET"` / `"get"` / `""` all map sensibly (`""` collapses to `Any`).
/// The URL template is canonicalised through `url_pattern::normalize` so
/// dynamic-segment variants (`{id}`, `:id`, `<id>`) compare equal across
/// languages.
fn extracted_routes_to_emissions(
    routes: &[crate::types::ExtractedRoute],
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<(u32, flow_emit::FlowEmission)> {
    let mut out = Vec::with_capacity(routes.len());
    for route in routes {
        if route.template.trim().is_empty() {
            continue;
        }
        let handler_line = symbols
            .get(route.handler_symbol_index)
            .map(|s| s.start_line)
            .unwrap_or(1);
        let method = flow_emit::HttpMethod::from_method_name(route.http_method.as_str());
        let name = crate::connectors::url_pattern::normalize(route.template.as_str());
        out.push((
            handler_line,
            flow_emit::FlowEmission::NamedChannel {
                kind: flow_emit::NamedChannelKind::HttpCall,
                name,
                role: flow_emit::ChannelRole::Consumer,
                method: Some(method),
            streaming: None,
            },
        ));
    }
    out
}

/// Flush extractor-time `FlowEmission`s into the resolve loop's accumulator.
/// Used for plugins whose flow detection is file-structure-based (GraphQL
/// SDL blocks, `.proto` service definitions) and runs at parse time rather
/// than during chain-walk resolver emission. The tuple shape matches
/// `extracted_routes_to_emissions` — `(file_path, line, emission)`.
fn plugin_flow_emissions_to_emissions(
    pf: &crate::types::ParsedFile,
    emissions: &mut Vec<(String, u32, flow_emit::FlowEmission)>,
) {
    for (line, emission) in &pf.plugin_flow_emissions {
        emissions.push((pf.path.clone(), *line, emission.clone()));
    }
}

/// Convert per-file `ExtractedDbSet` entries into `FlowEmission::DbEntity`
/// emissions. Used for languages whose model-table mapping is extracted
/// at the symbol layer (C# EF Core `DbSet<TEntity>` properties on a
/// DbContext). Each entry becomes a DbEntity row keyed by the table name
/// (or entity type name if the table name is empty), letting DbQuery
/// emissions for the same entity name pair against it via the pairer's
/// fuzzy name matching.
fn extracted_db_sets_to_emissions(
    db_sets: &[crate::types::ExtractedDbSet],
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<(u32, flow_emit::FlowEmission)> {
    let mut out = Vec::with_capacity(db_sets.len());
    for ds in db_sets {
        let line = symbols
            .get(ds.property_symbol_index)
            .map(|s| s.start_line)
            .unwrap_or(1);
        let table = if ds.table_name.is_empty() {
            None
        } else {
            Some(ds.table_name.clone())
        };
        out.push((
            line,
            flow_emit::FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: ds.entity_type.clone(),
                table_name_hint: table,
            },
        ));
    }
    out
}

/// Append a Consumer `NamedChannel { kind: HttpCall, .. }` emission for every
/// row in the `routes` table whose file is internal and whose template is
/// non-empty. Used for languages whose route extraction is implemented as a
/// project-wide `Connector` writing to the DB directly (Go chi/gin, Java
/// Spring `@GetMapping`, Elixir Phoenix scope blocks). The per-file
/// `pf.routes`-driven adapter handles the remaining languages (C# and TS
/// NestJS). Skips routes whose host file isn't on disk (deleted between
/// extract and resolve).
pub fn append_db_route_consumer_emissions(
    conn: &rusqlite::Connection,
    emissions: &mut Vec<(String, u32, flow_emit::FlowEmission)>,
) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT f.path,
                    r.http_method,
                    COALESCE(r.resolved_route, r.route_template) AS template,
                    r.line
             FROM routes r
             JOIN files f ON r.file_id = f.id
             WHERE f.origin = 'internal'",
        )
        .context("preparing routes-table read for flow adapter")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        let method_str: String = row.get(1)?;
        let template: String = row.get(2)?;
        let line: u32 = row.get(3).unwrap_or(1);
        if template.trim().is_empty() {
            continue;
        }
        let method = flow_emit::HttpMethod::from_method_name(method_str.as_str());
        let name = crate::connectors::url_pattern::normalize(template.as_str());
        emissions.push((
            path,
            line.max(1),
            flow_emit::FlowEmission::NamedChannel {
                kind: flow_emit::NamedChannelKind::HttpCall,
                name,
                role: flow_emit::ChannelRole::Consumer,
                method: Some(method),
            streaming: None,
            },
        ));
    }
    Ok(())
}

fn read_file_imports_from_db(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Vec<ImportEntry> {
    let sql = "SELECT i.imported_name, i.module_path, i.alias
               FROM imports i
               JOIN files f ON f.id = i.file_id
               WHERE f.path = ?1";
    let Ok(mut stmt) = conn.prepare_cached(sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map([file_path], |r| {
        Ok(ImportEntry {
            imported_name: r.get::<_, String>(0)?,
            module_path: r.get::<_, Option<String>>(1)?,
            alias: r.get::<_, Option<String>>(2)?,
            is_wildcard: false, // Not persisted; safe default.
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}
