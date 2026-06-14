// =============================================================================
// indexer/resolve/write_buf.rs — per-file write buffer + bulk SQL flush
//
// Pure data plumbing for the resolve loop: each rayon worker fills a
// `FileWriteBuf` of pending edges / external_refs / unresolved_refs /
// flow-emission tuples, then the main thread reduces them and calls
// `flush_resolve_buf` to bulk-insert via batched VALUES statements.
// `flush_flow_emissions` (the separate Producer/Consumer pairing pipeline)
// lives in flow_pair.rs.
// =============================================================================

use anyhow::{Context, Result};

use super::flow_emit::FlowEmission;

// Per-file output buffer. Each rayon worker fills its own; the main thread
// merges + bulk-writes after the parallel section. Avoids sharing the
// rusqlite Transaction across workers (it isn't `Sync`).
#[derive(Default)]
pub(super) struct FileWriteBuf {
    /// (source_id, target_id, kind, source_line, confidence, strategy)
    pub(super) edges: Vec<(i64, i64, &'static str, u32, f64, &'static str)>,
    /// (source_id, target_name, kind, source_line, namespace, package_id)
    pub(super) externals: Vec<(i64, String, &'static str, u32, String, Option<i64>)>,
    /// (source_id, target_name, kind, source_line, module, package_id, from_snippet)
    pub(super) unresolved: Vec<(
        i64,
        String,
        &'static str,
        u32,
        Option<String>,
        Option<i64>,
        bool,
    )>,
    /// Flow-edge emissions from resolver-detected patterns.
    /// Each entry: (file_path, source_line, emission).
    /// The file_path is resolved to a DB file_id during flush.
    pub(super) flow_emissions: Vec<(String, u32, FlowEmission)>,
    /// Return-type candidates harvested from `return <expr>` sites this pass:
    /// `(function_qname, function_db_id, resolved_yield_type_name)`. Not flushed
    /// to SQL — the orchestrator joins these per function (conflict → skip;
    /// qname owned by >1 function → skip) and gap-fills the cached index so
    /// callers read the inferred return (INFER-3/2). The db_id distinguishes
    /// two functions that share a qname across files.
    pub(super) inferred_returns: Vec<(String, i64, String)>,
}

impl FileWriteBuf {
    pub(super) fn merge(&mut self, mut other: Self) {
        self.edges.append(&mut other.edges);
        self.externals.append(&mut other.externals);
        self.unresolved.append(&mut other.unresolved);
        self.flow_emissions.append(&mut other.flow_emissions);
        self.inferred_returns.append(&mut other.inferred_returns);
    }

    /// Rewrite edge target ids through a synthetic→real id map. An edge whose
    /// target was a materialized external carries that symbol's synthetic id
    /// during the pass; once the materialized rows are written with real ids,
    /// this rebinds the targets so the FK-enforced edge flush succeeds.
    pub(super) fn remap_edge_targets(&mut self, remap: &std::collections::HashMap<i64, i64>) {
        if remap.is_empty() {
            return;
        }
        for e in &mut self.edges {
            if let Some(&real) = remap.get(&e.1) {
                e.1 = real;
            }
        }
    }
}

/// Speculative rows (unresolved + external refs) carried out of a resolve pass
/// so the demand loop persists them once after it converges. Opaque to the
/// orchestrator (full.rs); flushed via `flush_deferred_speculative`.
#[derive(Default)]
pub(crate) struct DeferredSpeculative {
    buf: FileWriteBuf,
}

impl DeferredSpeculative {
    /// Replace the held speculative rows with this pass's. The latest pass is
    /// authoritative — earlier passes' unresolved/external sets are superseded
    /// as refs resolve against newly-pulled external files — so this overwrites
    /// rather than appends. Moves the vecs out of `src` (cheap, no copy).
    pub(super) fn replace_from(&mut self, src: &mut FileWriteBuf) {
        self.buf.externals = std::mem::take(&mut src.externals);
        self.buf.unresolved = std::mem::take(&mut src.unresolved);
    }

    /// The held rows as a buffer ready for `flush_resolve_buf` (edges empty).
    pub(super) fn buf(&self) -> &FileWriteBuf {
        &self.buf
    }
}

/// Counters accumulated per file; reduced into the global ResolutionStats
/// after the parallel section. Excludes `chain_misses`, which are pushed
/// directly into the SymbolIndex's Mutex-protected accumulator by the
/// chain walker (already thread-safe).
#[derive(Default, Clone, Copy)]
pub(super) struct FileStats {
    pub(super) resolved: u64,
    pub(super) engine_resolved: u64,
    pub(super) unresolved: u64,
    pub(super) external: u64,
}

impl FileStats {
    pub(super) fn merge(&mut self, other: Self) {
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
pub(super) fn flush_resolve_buf(
    tx: &rusqlite::Transaction<'_>,
    buf: &FileWriteBuf,
    persist_speculative: bool,
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
            if i > 0 {
                s.push(',');
            }
            s.push('(');
            for j in 0..cols {
                if j > 0 {
                    s.push(',');
                }
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
    // Speculative — deferred to a single post-convergence flush on the demand
    // loop (persist_speculative=false on intermediate passes; see full.rs).
    if persist_speculative && !buf.externals.is_empty() {
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
    // Speculative — same deferral as external_refs above.
    if persist_speculative && !buf.unresolved.is_empty() {
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
