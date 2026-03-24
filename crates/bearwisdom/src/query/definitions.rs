// =============================================================================
// query/definitions.rs  —  go-to-definition query
//
// Given a symbol name (or qualified name), find where it is defined.
//
// Query strategies (in priority order):
//   1. Exact qualified_name match — fastest, highest confidence.
//   2. Exact name match — may return multiple results if the name is
//      ambiguous across files.  Caller may filter further by file or kind.
// =============================================================================

use crate::db::Database;
use crate::types::DefinitionResult;
use anyhow::{Context, Result};

/// Look up definition(s) for a symbol by name or qualified name.
///
/// `query` may be:
///   - A qualified name:  "Catalog.CatalogApi.MapCatalogApiV1" (exact match)
///   - A simple name:     "MapCatalogApiV1" (may return multiple hits)
///
/// Returns results ordered by confidence descending.
pub fn goto_definition(db: &Database, query: &str) -> Result<Vec<DefinitionResult>> {
    let conn = &db.conn;
    let mut results: Vec<DefinitionResult> = Vec::new();

    // --- Strategy 1: exact qualified name ---
    {
        let mut stmt = conn.prepare(
            "SELECT name, qualified_name, kind, f.path, s.line, s.col, s.signature
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.qualified_name = ?1
             ORDER BY s.line",
        ).context("Failed to prepare qualified name query")?;

        let rows = stmt.query_map([query], |row| {
            Ok(DefinitionResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                line: row.get(4)?,
                col: row.get(5)?,
                signature: row.get(6)?,
                confidence: 1.0,
            })
        }).context("Failed to execute qualified name query")?;

        for row in rows {
            results.push(row?);
        }
    }

    if !results.is_empty() {
        return Ok(results);
    }

    // --- Strategy 2: simple name ---
    // When the query contains a dot, treat the last segment as the name to
    // look up (fallback for partial qualified names).
    let simple_name = query.rsplit('.').next().unwrap_or(query);

    {
        let mut stmt = conn.prepare(
            "SELECT name, qualified_name, kind, f.path, s.line, s.col, s.signature
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1
             ORDER BY s.line",
        ).context("Failed to prepare simple name query")?;

        let rows = stmt.query_map([simple_name], |row| {
            Ok(DefinitionResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                line: row.get(4)?,
                col: row.get(5)?,
                signature: row.get(6)?,
                confidence: 0.7, // lower confidence — may be ambiguous
            })
        }).context("Failed to execute simple name query")?;

        for row in rows {
            results.push(row?);
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn insert_symbol(db: &Database, path: &str, name: &str, qname: &str, kind: &str, line: u32) -> i64 {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
             ON CONFLICT(path) DO NOTHING",
            [path],
        ).unwrap();
        let file_id: i64 = conn.query_row(
            "SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0)
        ).unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            rusqlite::params![file_id, name, qname, kind, line],
        ).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn goto_definition_by_qualified_name() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "Catalog.cs", "GetById", "Catalog.Service.GetById", "method", 10);

        let results = goto_definition(&db, "Catalog.Service.GetById").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "GetById");
        assert_eq!(results[0].confidence, 1.0);
    }

    #[test]
    fn goto_definition_by_simple_name() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "Catalog.cs", "GetById", "Catalog.Service.GetById", "method", 10);

        let results = goto_definition(&db, "GetById").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "GetById");
    }

    #[test]
    fn goto_definition_returns_empty_for_unknown() {
        let db = Database::open_in_memory().unwrap();
        let results = goto_definition(&db, "DoesNotExist").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn goto_definition_returns_multiple_for_ambiguous_name() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "a.cs", "Process", "NS1.Svc.Process", "method", 1);
        insert_symbol(&db, "b.cs", "Process", "NS2.Worker.Process", "method", 5);

        let results = goto_definition(&db, "Process").unwrap();
        assert_eq!(results.len(), 2);
    }
}
