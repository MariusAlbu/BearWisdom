// =============================================================================
// query/call_hierarchy.rs  —  incoming and outgoing call queries
//
// Answers two questions:
//   • "Who calls X?"   → incoming_calls(symbol_name)
//   • "What does X call?" → outgoing_calls(symbol_name)
//
// IDE-038 fix: the resolver stores explicit method-call expressions as
// kind='calls', but class/service usage (constructor injection, field access,
// method dispatch through a typed reference) is stored as kind='type_ref' or
// kind='instantiates'.  Restricting to kind='calls' alone leaves those
// relationships invisible in the call hierarchy, which is confusing for
// service-layer code where the primary dependency graph is TypeRef edges.
//
// The query now accepts all three kinds: calls, type_ref, instantiates.
// This is the pragmatic fallback described in the issue — it surfaces all
// symbol usage as "callers", not just explicit call expressions.
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// One item in a call hierarchy — either a caller of X or a callee of X.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub qualified_name: String,
    /// Symbol kind string, e.g. "method", "function", "class".
    pub kind: String,
    pub file_path: String,
    /// 1-based source line of the call site (where the call appears in source).
    /// This is `edges.source_line`, which is 0 if the extractor did not record it.
    pub line: u32,
}

// ---------------------------------------------------------------------------
// Helper — look up target symbol ID(s)
// ---------------------------------------------------------------------------

/// Resolve `symbol_name` (simple or qualified) to a list of symbol IDs.
/// Returns an empty vec if no match is found.
fn resolve_ids(db: &Database, symbol_name: &str) -> Result<Vec<i64>> {
    let conn = &db.conn;
    let ids = if symbol_name.contains('.') {
        // Qualified name: expect exactly one match.
        let mut stmt = conn.prepare(
            "SELECT id FROM symbols WHERE qualified_name = ?1"
        ).context("Failed to prepare qualified lookup")?;
        let rows = stmt.query_map([symbol_name], |r| r.get(0))
            .context("Failed to query qualified lookup")?;
        rows.filter_map(|r| r.ok()).collect()
    } else {
        // Simple name: may be ambiguous — return all matches.
        let mut stmt = conn.prepare(
            "SELECT id FROM symbols WHERE name = ?1"
        ).context("Failed to prepare simple lookup")?;
        let rows = stmt.query_map([symbol_name], |r| r.get(0))
            .context("Failed to query simple lookup")?;
        rows.filter_map(|r| r.ok()).collect()
    };
    Ok(ids)
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Find all symbols that call `symbol_name` (incoming calls).
///
/// Returns up to `limit` results sorted by file path and source line.
/// Pass `limit = 0` for unlimited results.
pub fn incoming_calls(
    db: &Database,
    symbol_name: &str,
    limit: usize,
) -> Result<Vec<CallHierarchyItem>> {
    let target_ids = resolve_ids(db, symbol_name)?;
    if target_ids.is_empty() {
        return Ok(vec![]);
    }

    let conn = &db.conn;
    let limit_clause = if limit > 0 { format!("LIMIT {limit}") } else { String::new() };
    let mut results = Vec::new();

    for target_id in &target_ids {
        // IDE-038: include calls, type_ref, and instantiates edges so that
        // service-layer usage (dependency injection, typed references) is
        // visible alongside explicit call expressions.
        let sql = format!(
            "SELECT s.name,
                    s.qualified_name,
                    s.kind,
                    f.path           AS file_path,
                    COALESCE(e.source_line, 0) AS line
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE e.target_id = ?1
               AND e.kind IN ('calls', 'type_ref', 'instantiates')
             ORDER BY f.path, e.source_line
             {limit_clause}"
        );

        let mut stmt = conn.prepare(&sql)
            .context("Failed to prepare incoming_calls query")?;

        let rows = stmt.query_map([target_id], |row| {
            Ok(CallHierarchyItem {
                name:           row.get(0)?,
                qualified_name: row.get(1)?,
                kind:           row.get(2)?,
                file_path:      row.get(3)?,
                line:           row.get(4)?,
            })
        }).context("Failed to execute incoming_calls query")?;

        for row in rows {
            results.push(row.context("Failed to read incoming_calls row")?);
        }
    }

    // Stable sort: file path then line.
    results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    if limit > 0 && results.len() > limit {
        results.truncate(limit);
    }

    Ok(results)
}

/// Find all symbols that `symbol_name` calls (outgoing calls).
///
/// Returns up to `limit` results sorted by file path and source line.
/// Pass `limit = 0` for unlimited results.
pub fn outgoing_calls(
    db: &Database,
    symbol_name: &str,
    limit: usize,
) -> Result<Vec<CallHierarchyItem>> {
    let source_ids = resolve_ids(db, symbol_name)?;
    if source_ids.is_empty() {
        return Ok(vec![]);
    }

    let conn = &db.conn;
    let limit_clause = if limit > 0 { format!("LIMIT {limit}") } else { String::new() };
    let mut results = Vec::new();

    for source_id in &source_ids {
        // IDE-038: include calls, type_ref, and instantiates edges so that
        // service-layer dependencies are visible alongside explicit calls.
        let sql = format!(
            "SELECT s.name,
                    s.qualified_name,
                    s.kind,
                    f.path           AS file_path,
                    COALESCE(e.source_line, 0) AS line
             FROM edges e
             JOIN symbols s ON s.id = e.target_id
             JOIN files   f ON f.id = s.file_id
             WHERE e.source_id = ?1
               AND e.kind IN ('calls', 'type_ref', 'instantiates')
             ORDER BY f.path, e.source_line
             {limit_clause}"
        );

        let mut stmt = conn.prepare(&sql)
            .context("Failed to prepare outgoing_calls query")?;

        let rows = stmt.query_map([source_id], |row| {
            Ok(CallHierarchyItem {
                name:           row.get(0)?,
                qualified_name: row.get(1)?,
                kind:           row.get(2)?,
                file_path:      row.get(3)?,
                line:           row.get(4)?,
            })
        }).context("Failed to execute outgoing_calls query")?;

        for row in rows {
            results.push(row.context("Failed to read outgoing_calls row")?);
        }
    }

    results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    if limit > 0 && results.len() > limit {
        results.truncate(limit);
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "call_hierarchy_tests.rs"]
mod tests;
