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
    let conn = &db.conn;

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
        db.conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'foo', 'mod::foo', 'function', 5, 0)",
            [file_id],
        ).unwrap();
        let sym_id = db.conn.last_insert_rowid();

        db.conn.execute(
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
        db.conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'caller', 'mod::caller', 'function', 1, 0)",
            [file_id],
        ).unwrap();
        let src_id = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'callee', 'mod::callee', 'function', 20, 0)",
            [file_id],
        ).unwrap();
        let tgt_id = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'calls', 5, 0.50)",
            rusqlite::params![src_id, tgt_id],
        ).unwrap();

        let result = get_diagnostics(&db, "src/a.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
        assert_eq!(result.low_confidence_count, 1);
        assert_eq!(result.diagnostics[0].kind, DiagnosticKind::LowConfidenceEdge);
        assert_eq!(result.diagnostics[0].confidence, Some(0.50));
    }
}
