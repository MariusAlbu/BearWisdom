// =============================================================================
// engine/flush.rs — bulk DB write for resolution output
//
// Row-type aliases for the resolve pass output plus the single transaction
// that writes edges / unresolved_refs / ref_resolutions. The full pass clears
// all resolution tables first; the incremental pass inserts only. The logic
// mirrors write_buf::flush_resolve_buf with persist_speculative=true.
// =============================================================================

use anyhow::{Context, Result};

use crate::db::Database;

/// A resolved edge row: (source_id, target_id, kind, source_line, confidence, strategy).
pub(super) type Edge = (i64, i64, &'static str, u32, f64, &'static str);
/// An unresolved-ref row: (source_id, target_name, kind, source_line, module,
/// package_id, from_snippet, drained, cause_symbol_id, cause_kind).
pub(super) type Unresolved = (
    i64,
    String,
    &'static str,
    u32,
    Option<String>,
    Option<i64>,
    bool,
    bool,
    Option<i64>,
    Option<&'static str>,
);
/// Per-reference evidence. Byte offsets survive independently of the legacy
/// line/column fields (some extractors still emit column zero).
#[derive(Debug)]
pub(super) struct RefLog {
    source_id: i64,
    target_name: String,
    kind: &'static str,
    line: u32,
    col: u32,
    byte_offset: u32,
    selector_byte: u32,
    outcome: &'static str,
    target_id: Option<i64>,
    confidence: Option<f64>,
    strategy: Option<&'static str>,
}

impl RefLog {
    pub(super) fn resolved(
        source_id: i64,
        r: &crate::types::ExtractedRef,
        info: &super::contract::SymbolInfo,
    ) -> Self {
        Self {
            outcome: "resolved",
            target_id: Some(info.target_symbol_id),
            confidence: Some(info.confidence),
            strategy: Some(info.strategy),
            ..Self::unresolved(source_id, r, false)
        }
    }

    pub(super) fn unresolved(
        source_id: i64,
        r: &crate::types::ExtractedRef,
        drained: bool,
    ) -> Self {
        Self {
            source_id,
            target_name: r.target_name.clone(),
            kind: r.kind.into(),
            line: r.line,
            col: r.col,
            byte_offset: r.byte_offset,
            selector_byte: r
                .chain
                .as_ref()
                .and_then(|c| c.segments.last())
                .map(|s| s.byte_offset)
                .unwrap_or(r.byte_offset),
            outcome: if drained { "drained" } else { "unresolved" },
            target_id: None,
            confidence: None,
            strategy: None,
        }
    }
}

pub(super) fn flush_to_db(
    db: &mut Database,
    edges: &[(i64, i64, &'static str, u32, f64, &'static str)],
    unresolved: &[Unresolved],
    ref_log: &[RefLog],
    censuses: &[super::occurrence_census::FileCensus<'_>],
    clear_existing: bool,
) -> Result<()> {
    use rusqlite::types::Value;

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin single-pass resolution transaction")?;

    // The full pass replaces all four tables; the incremental pass inserts only
    // (the changed files' old rows were already dropped upstream when their
    // symbols were rewritten, and the rest of the tables must survive).
    if clear_existing {
        tx.execute("DELETE FROM edges", [])
            .context("Failed to clear edges")?;
        tx.execute("DELETE FROM unresolved_refs", [])
            .context("Failed to clear unresolved_refs")?;
        tx.execute("DELETE FROM ref_resolutions", [])
            .context("Failed to clear ref_resolutions")?;
    }

    const EDGE_CHUNK: usize = 256;
    const UNRESOLVED_CHUNK: usize = 256;
    const REF_LOG_CHUNK: usize = 256;

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
    if !edges.is_empty() {
        let mut start = 0;
        while start < edges.len() {
            let end = (start + EDGE_CHUNK).min(edges.len());
            let rows = end - start;
            let sql = format!(
                "INSERT OR IGNORE INTO edges \
                 (source_id, target_id, kind, source_line, confidence, strategy) \
                 VALUES {}",
                placeholders(rows, 6),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 6);
            for (sid, tid, kind, line, conf, strat) in &edges[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Integer(*tid));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(Value::Real(*conf));
                params.push(Value::Text((*strat).to_string()));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare edges insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute edges insert")?;
            start = end;
        }
    }

    // Unresolved refs: (source_id, target_name, kind, source_line, module,
    // package_id, from_snippet, drained, cause_symbol_id, cause_kind)
    if !unresolved.is_empty() {
        let mut start = 0;
        while start < unresolved.len() {
            let end = (start + UNRESOLVED_CHUNK).min(unresolved.len());
            let rows = end - start;
            let sql = format!(
                "INSERT OR IGNORE INTO unresolved_refs \
                 (source_id, target_name, kind, source_line, module, package_id, from_snippet, drained, \
                  cause_symbol_id, cause_kind) \
                 VALUES {}",
                placeholders(rows, 10),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 10);
            for (
                sid,
                name,
                kind,
                line,
                module,
                pkg,
                from_snippet,
                drained,
                cause_symbol_id,
                cause_kind,
            ) in &unresolved[start..end]
            {
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
                params.push(Value::Integer(if *drained { 1 } else { 0 }));
                params.push(match cause_symbol_id {
                    Some(v) => Value::Integer(*v),
                    None => Value::Null,
                });
                params.push(match cause_kind {
                    Some(s) => Value::Text((*s).to_string()),
                    None => Value::Null,
                });
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare unresolved_refs insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute unresolved_refs insert")?;
            start = end;
        }
    }

    // Ref resolution log: (source_id, target_name, kind, source_line,
    // source_col, outcome, target_id, confidence, strategy, source_byte)
    if !ref_log.is_empty() {
        let mut start = 0;
        while start < ref_log.len() {
            let end = (start + REF_LOG_CHUNK).min(ref_log.len());
            let rows = end - start;
            let sql = format!(
                "INSERT INTO ref_resolutions \
                 (source_id, target_name, kind, source_line, source_col, outcome, target_id, confidence, strategy, source_byte, source_selector_byte) \
                 VALUES {}",
                placeholders(rows, 11),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 11);
            for row in &ref_log[start..end] {
                params.push(Value::Integer(row.source_id));
                params.push(Value::Text(row.target_name.clone()));
                params.push(Value::Text(row.kind.to_string()));
                params.push(Value::Integer(row.line as i64));
                params.push(Value::Integer(row.col as i64));
                params.push(Value::Text(row.outcome.to_string()));
                params.push(match row.target_id {
                    Some(v) => Value::Integer(v),
                    None => Value::Null,
                });
                params.push(match row.confidence {
                    Some(v) => Value::Real(v),
                    None => Value::Null,
                });
                params.push(match row.strategy {
                    Some(s) => Value::Text(s.to_string()),
                    None => Value::Null,
                });
                params.push(Value::Integer(row.byte_offset as i64));
                params.push(Value::Integer(row.selector_byte as i64));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare ref_resolutions insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute ref_resolutions insert")?;
            start = end;
        }
    }

    super::occurrence_census::persist(&tx, censuses, clear_existing)?;
    tx.commit()
        .context("Failed to commit single-pass resolution transaction")?;
    Ok(())
}
