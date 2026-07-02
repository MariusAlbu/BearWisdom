// =============================================================================
// query/ref_snapshot.rs — per-ref resolution snapshot export/import
//
// Exports every row of `ref_resolutions` as a content-keyed, deterministically
// sorted JSONL stream: key = (file, line, col, target, kind), value = the
// ref's resolution outcome. Two snapshots taken across index runs can be
// diffed at ref-site granularity by `query::resolve_diff` to detect flips
// (resolved <-> unresolved) and silent retargeting (same key, different
// target) that edge-count metrics alone cannot see.
// =============================================================================

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::query::QueryResult;

/// One ref site's resolution outcome, keyed by `(file, line, col, target, kind)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefSnapshotEntry {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub target: String,
    pub kind: String,
    /// `"resolved"` | `"unresolved"` | `"drained"`.
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target_qname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confidence: Option<f64>,
}

impl RefSnapshotEntry {
    /// The content key two snapshots are diffed on: identifies "the same ref
    /// site" across two independent index runs.
    pub fn key(&self) -> (&str, u32, u32, &str, &str) {
        (&self.file, self.line, self.col, &self.target, &self.kind)
    }
}

/// Export every `ref_resolutions` row for the current DB state, joined to its
/// source file and (when resolved) target symbol, sorted deterministically by
/// key so the output is stable across runs with identical resolution outcomes.
pub fn export_ref_snapshot(db: &Database) -> QueryResult<Vec<RefSnapshotEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT f.path, rr.source_line, rr.source_col, rr.target_name, rr.kind, \
                    rr.outcome, t.qualified_name, tf.path, rr.strategy, rr.confidence \
             FROM ref_resolutions rr \
             JOIN symbols s ON s.id = rr.source_id \
             JOIN files f ON f.id = s.file_id \
             LEFT JOIN symbols t ON t.id = rr.target_id \
             LEFT JOIN files tf ON tf.id = t.file_id \
             ORDER BY f.path, rr.source_line, rr.source_col, rr.target_name, rr.kind",
        )
        .context("ref_snapshot: prepare export query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RefSnapshotEntry {
                file: row.get(0)?,
                line: row.get(1)?,
                col: row.get(2)?,
                target: row.get(3)?,
                kind: row.get(4)?,
                outcome: row.get(5)?,
                target_qname: row.get(6)?,
                target_file: row.get(7)?,
                strategy: row.get(8)?,
                confidence: row.get(9)?,
            })
        })
        .context("ref_snapshot: execute export query")?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.context("ref_snapshot: read row")?);
    }
    Ok(out)
}

/// Serialize `rows` as JSONL (one compact JSON object per line) to `w`.
pub fn write_snapshot_jsonl<W: Write>(rows: &[RefSnapshotEntry], w: &mut W) -> QueryResult<()> {
    for row in rows {
        let line = serde_json::to_string(row).context("ref_snapshot: serialize row")?;
        writeln!(w, "{line}").context("ref_snapshot: write row")?;
    }
    Ok(())
}

/// Read a JSONL snapshot file written by [`write_snapshot_jsonl`] back into rows.
/// Blank lines are skipped.
pub fn read_snapshot_jsonl(path: &Path) -> QueryResult<Vec<RefSnapshotEntry>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("ref_snapshot: failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("ref_snapshot: read line {}", i + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: RefSnapshotEntry = serde_json::from_str(trimmed)
            .with_context(|| format!("{}:{}: invalid snapshot row", path.display(), i + 1))?;
        out.push(entry);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "ref_snapshot_tests.rs"]
mod tests;
