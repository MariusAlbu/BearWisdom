// =============================================================================
// query/diagnostics.rs  —  per-file diagnostic surface
//
// Surfaces two kinds of issues for a given file:
//   1. Unresolved symbols — references that couldn't be matched to a symbol ID.
//   2. Low-confidence edges — edges below a confidence threshold (heuristic matches).
//
// Designed for IDE squiggly underlines and LLM-driven code analysis.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// The kind of diagnostic issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    /// A reference that could not be resolved to any known symbol.
    UnresolvedSymbol,
    /// An edge with confidence below the threshold (heuristic match).
    LowConfidenceEdge,
}

/// A single diagnostic for a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// 1-based line number where the issue occurs.
    pub line: u32,
    /// Diagnostic category.
    pub kind: DiagnosticKind,
    /// Human-readable description.
    pub message: String,
    /// The unresolved name or low-confidence target (for tool tips).
    pub target_name: Option<String>,
    /// Confidence value (only for LowConfidenceEdge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Edge kind (only for LowConfidenceEdge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_kind: Option<String>,
}

/// Summary returned by `get_diagnostics`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiagnostics {
    pub file_path: String,
    pub unresolved_count: u32,
    pub low_confidence_count: u32,
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Default confidence threshold below which edges are flagged.
pub const LOW_CONFIDENCE_THRESHOLD: f64 = 0.80;

/// Return diagnostics for a single file.
///
/// `file_path` is relative to the project root (as stored in the `files` table).
/// `confidence_threshold` controls when edges are flagged (default 0.80).
pub fn get_diagnostics(
    db: &Database,
    file_path: &str,
    confidence_threshold: f64,
) -> QueryResult<FileDiagnostics> {
    let _timer = db.timer("diagnostics");
    let conn = db.conn();

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // --- 1. Unresolved references ---
    {
        let mut stmt = conn.prepare(
            "SELECT ur.source_line, ur.target_name, ur.kind
             FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1
             ORDER BY ur.source_line",
        ).context("diagnostics: prepare unresolved query")?;

        let rows = stmt.query_map([file_path], |row| {
            let line: u32 = row.get::<_, Option<u32>>(0)?.unwrap_or(0);
            let target_name: String = row.get(1)?;
            let kind: String = row.get(2)?;
            Ok(Diagnostic {
                line,
                kind: DiagnosticKind::UnresolvedSymbol,
                message: format!("Unresolved {kind}: '{target_name}'"),
                target_name: Some(target_name),
                confidence: None,
                edge_kind: None,
            })
        }).context("diagnostics: execute unresolved query")?;

        for row in rows {
            if let Ok(d) = row {
                diagnostics.push(d);
            }
        }
    }

    let unresolved_count = diagnostics.len() as u32;

    // --- 2. Low-confidence edges ---
    {
        let mut stmt = conn.prepare(
            "SELECT e.source_line, e.kind, e.confidence,
                    ts.name AS target_name, ts.qualified_name
             FROM edges e
             JOIN symbols ss ON ss.id = e.source_id
             JOIN symbols ts ON ts.id = e.target_id
             JOIN files f ON f.id = ss.file_id
             WHERE f.path = ?1
               AND e.confidence < ?2
             ORDER BY e.source_line",
        ).context("diagnostics: prepare low-confidence query")?;

        let rows = stmt.query_map(
            rusqlite::params![file_path, confidence_threshold],
            |row| {
                let line: u32 = row.get::<_, Option<u32>>(0)?.unwrap_or(0);
                let edge_kind: String = row.get(1)?;
                let confidence: f64 = row.get(2)?;
                let target_name: String = row.get(3)?;
                let target_qn: String = row.get(4)?;
                Ok(Diagnostic {
                    line,
                    kind: DiagnosticKind::LowConfidenceEdge,
                    message: format!(
                        "Low-confidence {edge_kind} → '{target_qn}' ({:.0}%)",
                        confidence * 100.0,
                    ),
                    target_name: Some(target_name),
                    confidence: Some(confidence),
                    edge_kind: Some(edge_kind),
                })
            },
        ).context("diagnostics: execute low-confidence query")?;

        for row in rows {
            if let Ok(d) = row {
                diagnostics.push(d);
            }
        }
    }

    let low_confidence_count = diagnostics.len() as u32 - unresolved_count;

    // Sort all diagnostics by line number.
    diagnostics.sort_by_key(|d| d.line);

    Ok(FileDiagnostics {
        file_path: file_path.to_string(),
        unresolved_count,
        low_confidence_count,
        diagnostics,
    })
}

// ---------------------------------------------------------------------------
// Project-wide low-confidence roll-up
// ---------------------------------------------------------------------------

/// One bucket in the low-confidence summary — a (strategy, kind) pair with
/// its count and a range of observed confidence values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowConfidenceBucket {
    /// Resolver strategy string (e.g. "heuristic_name_kind", "ts_chain_resolution").
    /// `None` for edges written before strategy tracking landed, or for
    /// directly-inserted edges (SCIP import, tests).
    pub strategy: Option<String>,
    /// Edge kind (`calls`, `type_ref`, etc.).
    pub kind: String,
    pub count: u64,
    pub min_confidence: f64,
    pub max_confidence: f64,
}

/// Project-wide summary of low-confidence edges grouped by strategy and kind.
///
/// Used to audit the resolution pipeline — spot the largest sources of
/// heuristic edges and prioritize improvements. Rows sort by count desc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowConfidenceReport {
    pub threshold: f64,
    pub total: u64,
    pub buckets: Vec<LowConfidenceBucket>,
}

/// Return a roll-up of edges with `confidence < threshold`, grouped by
/// `(strategy, kind)`. Edges from the `external` file origin are excluded
/// so external-package noise doesn't dominate the view.
pub fn low_confidence_edges(
    db: &Database,
    threshold: f64,
) -> QueryResult<LowConfidenceReport> {
    let _timer = db.timer("low_confidence_edges");
    let conn = db.conn();

    let mut stmt = conn
        .prepare(
            "SELECT e.strategy,
                    e.kind,
                    COUNT(*)          AS cnt,
                    MIN(e.confidence) AS mn,
                    MAX(e.confidence) AS mx
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE e.confidence < ?1
               AND f.origin = 'internal'
             GROUP BY e.strategy, e.kind
             ORDER BY cnt DESC, e.strategy, e.kind",
        )
        .context("low_confidence_edges: prepare roll-up")?;

    let rows = stmt
        .query_map([threshold], |row| {
            Ok(LowConfidenceBucket {
                strategy: row.get::<_, Option<String>>(0)?,
                kind: row.get::<_, String>(1)?,
                count: row.get::<_, i64>(2)? as u64,
                min_confidence: row.get::<_, f64>(3)?,
                max_confidence: row.get::<_, f64>(4)?,
            })
        })
        .context("low_confidence_edges: execute roll-up")?;

    let buckets: Vec<LowConfidenceBucket> = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("low_confidence_edges: collect rows")?;
    let total = buckets.iter().map(|b| b.count).sum();

    Ok(LowConfidenceReport {
        threshold,
        total,
        buckets,
    })
}

// ---------------------------------------------------------------------------
// Workspace-wide diagnostics roll-up
// ---------------------------------------------------------------------------

/// One row in the workspace diagnostics ranking — a file with its unresolved
/// and low-confidence counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiagnosticSummary {
    pub file_path: String,
    pub language: String,
    pub unresolved_count: u32,
    pub low_confidence_count: u32,
}

/// Workspace-wide diagnostics report — ranks files by leakage so callers
/// can ask "where is the worst leakage?" in one query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDiagnostics {
    pub threshold: f64,
    pub total_unresolved: u64,
    pub total_low_confidence: u64,
    /// Files with the most unresolved refs, descending. Capped at `top_n`.
    pub top_files_by_unresolved: Vec<FileDiagnosticSummary>,
    /// Files with the most low-confidence edges, descending. Capped at `top_n`.
    pub top_files_by_low_confidence: Vec<FileDiagnosticSummary>,
}

/// Aggregate diagnostics across the whole indexed workspace.
///
/// Returns the top N files ranked by unresolved-ref count and the top N
/// files ranked by low-confidence-edge count. Both rankings are restricted
/// to `files.origin = 'internal'` so external dependency noise doesn't
/// dominate. `threshold` is applied to the low-confidence pass.
pub fn workspace_diagnostics(
    db: &Database,
    top_n: u32,
    threshold: f64,
) -> QueryResult<WorkspaceDiagnostics> {
    let _timer = db.timer("workspace_diagnostics");
    let conn = db.conn();

    let total_unresolved: u64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal'",
            [],
            |r| r.get::<_, i64>(0).map(|n| n as u64),
        )
        .context("workspace_diagnostics: total unresolved")?;

    let total_low_confidence: u64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE e.confidence < ?1
               AND f.origin = 'internal'",
            [threshold],
            |r| r.get::<_, i64>(0).map(|n| n as u64),
        )
        .context("workspace_diagnostics: total low-confidence")?;

    let top_files_by_unresolved: Vec<FileDiagnosticSummary> = {
        let mut stmt = conn
            .prepare(
                "SELECT f.path,
                        f.language,
                        COUNT(ur.source_id) AS unresolved_count,
                        (SELECT COUNT(*) FROM edges e2
                         JOIN symbols s2 ON s2.id = e2.source_id
                         WHERE s2.file_id = f.id
                           AND e2.confidence < ?1) AS low_conf_count
                 FROM unresolved_refs ur
                 JOIN symbols s ON s.id = ur.source_id
                 JOIN files   f ON f.id = s.file_id
                 WHERE f.origin = 'internal'
                 GROUP BY f.id, f.path, f.language
                 ORDER BY unresolved_count DESC, f.path
                 LIMIT ?2",
            )
            .context("workspace_diagnostics: prepare top-by-unresolved")?;

        let rows = stmt
            .query_map(rusqlite::params![threshold, top_n as i64], |row| {
                Ok(FileDiagnosticSummary {
                    file_path: row.get(0)?,
                    language: row.get(1)?,
                    unresolved_count: row.get::<_, i64>(2)? as u32,
                    low_confidence_count: row.get::<_, i64>(3)? as u32,
                })
            })
            .context("workspace_diagnostics: execute top-by-unresolved")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("workspace_diagnostics: collect top-by-unresolved")?
    };

    let top_files_by_low_confidence: Vec<FileDiagnosticSummary> = {
        let mut stmt = conn
            .prepare(
                "SELECT f.path,
                        f.language,
                        (SELECT COUNT(*) FROM unresolved_refs ur2
                         JOIN symbols s2 ON s2.id = ur2.source_id
                         WHERE s2.file_id = f.id) AS unresolved_count,
                        COUNT(e.source_id) AS low_conf_count
                 FROM edges e
                 JOIN symbols s ON s.id = e.source_id
                 JOIN files   f ON f.id = s.file_id
                 WHERE e.confidence < ?1
                   AND f.origin = 'internal'
                 GROUP BY f.id, f.path, f.language
                 ORDER BY low_conf_count DESC, f.path
                 LIMIT ?2",
            )
            .context("workspace_diagnostics: prepare top-by-low-confidence")?;

        let rows = stmt
            .query_map(rusqlite::params![threshold, top_n as i64], |row| {
                Ok(FileDiagnosticSummary {
                    file_path: row.get(0)?,
                    language: row.get(1)?,
                    unresolved_count: row.get::<_, i64>(2)? as u32,
                    low_confidence_count: row.get::<_, i64>(3)? as u32,
                })
            })
            .context("workspace_diagnostics: execute top-by-low-confidence")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("workspace_diagnostics: collect top-by-low-confidence")?
    };

    Ok(WorkspaceDiagnostics {
        threshold,
        total_unresolved,
        total_low_confidence,
        top_files_by_unresolved,
        top_files_by_low_confidence,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_empty_file_returns_empty_diagnostics() {
        let db = Database::open_in_memory().unwrap();
        let result = get_diagnostics(&db, "nonexistent.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
        assert_eq!(result.unresolved_count, 0);
        assert_eq!(result.low_confidence_count, 0);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_unresolved_refs_surfaced() {
        let db = Database::open_in_memory().unwrap();
        db.conn().execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'foo', 'mod::foo', 'function', 5, 0)",
            [file_id],
        ).unwrap();
        let sym_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line)
             VALUES (?1, 'Bar', 'type_ref', 8)",
            [sym_id],
        ).unwrap();

        let result = get_diagnostics(&db, "src/a.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
        assert_eq!(result.unresolved_count, 1);
        assert_eq!(result.diagnostics[0].kind, DiagnosticKind::UnresolvedSymbol);
        assert_eq!(result.diagnostics[0].line, 8);
        assert_eq!(result.diagnostics[0].target_name.as_deref(), Some("Bar"));
    }

    #[test]
    fn test_low_confidence_edges_surfaced() {
        let db = Database::open_in_memory().unwrap();
        db.conn().execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'caller', 'mod::caller', 'function', 1, 0)",
            [file_id],
        ).unwrap();
        let src_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'callee', 'mod::callee', 'function', 20, 0)",
            [file_id],
        ).unwrap();
        let tgt_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'calls', 5, 0.50)",
            rusqlite::params![src_id, tgt_id],
        ).unwrap();

        let result = get_diagnostics(&db, "src/a.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
        assert_eq!(result.low_confidence_count, 1);
        assert_eq!(result.diagnostics[0].kind, DiagnosticKind::LowConfidenceEdge);
        assert_eq!(result.diagnostics[0].confidence, Some(0.50));
    }

    /// Seed two edges at different confidences + strategies, verify the
    /// project-wide roll-up groups them correctly.
    fn seed_edge(
        db: &Database,
        file_id: i64,
        src_name: &str,
        tgt_name: &str,
        confidence: f64,
        strategy: Option<&str>,
    ) {
        let conn = db.conn();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, 'function', 1, 0)",
            rusqlite::params![file_id, src_name, format!("mod::{src_name}")],
        )
        .unwrap();
        let src_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, 'function', 5, 0)",
            rusqlite::params![file_id, tgt_name, format!("mod::{tgt_name}")],
        )
        .unwrap();
        let tgt_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence, strategy)
             VALUES (?1, ?2, 'calls', 1, ?3, ?4)",
            rusqlite::params![src_id, tgt_id, confidence, strategy],
        )
        .unwrap();
    }

    #[test]
    fn low_confidence_edges_buckets_by_strategy_and_kind() {
        let db = Database::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed, origin)
                 VALUES ('src/a.rs', 'h', 'rust', 0, 'internal')",
                [],
            )
            .unwrap();
        let file_id = db.conn().last_insert_rowid();

        seed_edge(&db, file_id, "a1", "a2", 0.50, Some("heuristic_name_kind"));
        seed_edge(&db, file_id, "b1", "b2", 0.35, Some("heuristic_name_kind"));
        seed_edge(&db, file_id, "c1", "c2", 0.95, Some("ts_chain_resolution"));
        seed_edge(&db, file_id, "d1", "d2", 1.00, Some("csharp_same_namespace"));

        let report = low_confidence_edges(&db, LOW_CONFIDENCE_THRESHOLD).unwrap();
        // 0.50, 0.35, 0.95 all < 0.80? No — 0.95 > 0.80, so only 0.50/0.35
        // actually fall under the default threshold.
        assert_eq!(report.total, 2);
        assert_eq!(report.buckets.len(), 1);
        let b = &report.buckets[0];
        assert_eq!(b.strategy.as_deref(), Some("heuristic_name_kind"));
        assert_eq!(b.kind, "calls");
        assert_eq!(b.count, 2);
        assert!((b.min_confidence - 0.35).abs() < 1e-9);
        assert!((b.max_confidence - 0.50).abs() < 1e-9);
    }

    #[test]
    fn low_confidence_edges_excludes_external_origin() {
        let db = Database::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed, origin)
                 VALUES ('ext/pkg/x.ts', 'h', 'typescript', 0, 'external')",
                [],
            )
            .unwrap();
        let file_id = db.conn().last_insert_rowid();
        seed_edge(&db, file_id, "x1", "x2", 0.50, Some("heuristic_name_kind"));

        let report = low_confidence_edges(&db, LOW_CONFIDENCE_THRESHOLD).unwrap();
        assert_eq!(report.total, 0);
    }
}
