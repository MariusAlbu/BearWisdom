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
#[path = "definitions_tests.rs"]
mod tests;
