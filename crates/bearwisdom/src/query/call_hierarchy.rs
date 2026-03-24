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
mod tests {
    use super::*;
    use crate::db::Database;

    /// Build a small graph: Caller → Service, Service → Db.
    fn setup(db: &Database) -> (i64, i64, i64) {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('a.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        for (name, qname, line) in [("Caller", "NS.Caller", 1i64), ("Service", "NS.Service", 10), ("Db", "NS.Db", 20)] {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'method', ?4, 0)",
                rusqlite::params![fid, name, qname, line],
            ).unwrap();
        }

        let caller:  i64 = conn.query_row("SELECT id FROM symbols WHERE name='Caller'",  [], |r| r.get(0)).unwrap();
        let service: i64 = conn.query_row("SELECT id FROM symbols WHERE name='Service'", [], |r| r.get(0)).unwrap();
        let db_sym:  i64 = conn.query_row("SELECT id FROM symbols WHERE name='Db'",      [], |r| r.get(0)).unwrap();

        conn.execute("INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 5, 1.0)",  rusqlite::params![caller, service]).unwrap();
        conn.execute("INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 15, 1.0)", rusqlite::params![service, db_sym]).unwrap();

        (caller, service, db_sym)
    }

    #[test]
    fn incoming_calls_finds_caller_of_service() {
        let db = Database::open_in_memory().unwrap();
        setup(&db);

        let items = incoming_calls(&db, "Service", 0).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Caller");
        assert_eq!(items[0].line, 5);
    }

    #[test]
    fn outgoing_calls_finds_callee_of_service() {
        let db = Database::open_in_memory().unwrap();
        setup(&db);

        let items = outgoing_calls(&db, "Service", 0).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Db");
        assert_eq!(items[0].line, 15);
    }

    #[test]
    fn incoming_calls_returns_empty_for_root() {
        let db = Database::open_in_memory().unwrap();
        setup(&db);

        // Caller has no callers.
        let items = incoming_calls(&db, "Caller", 0).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn outgoing_calls_returns_empty_for_leaf() {
        let db = Database::open_in_memory().unwrap();
        setup(&db);

        // Db calls nothing.
        let items = outgoing_calls(&db, "Db", 0).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn call_hierarchy_respects_limit() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('b.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'Tgt', 'NS.Tgt', 'method', 1, 0)",
            [fid],
        ).unwrap();
        let tgt: i64 = conn.last_insert_rowid();

        for i in 0..5i64 {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'U', ?, 'method', ?3, 0)",
                rusqlite::params![fid, format!("NS.U{i}"), i + 10],
            ).unwrap();
            let uid: i64 = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', ?3, 1.0)",
                rusqlite::params![uid, tgt, i],
            ).unwrap();
        }

        let items = incoming_calls(&db, "Tgt", 2).unwrap();
        assert_eq!(items.len(), 2, "Limit should be respected");
    }

    // IDE-038: type_ref edges ARE now included in the call hierarchy so that
    // service-layer usage (dependency injection, typed references) is visible.
    #[test]
    fn type_ref_edges_appear_in_call_hierarchy() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('c.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        for (name, qname) in [("Src", "NS.Src"), ("Dst", "NS.Dst")] {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'class', 1, 0)",
                rusqlite::params![fid, name, qname],
            ).unwrap();
        }
        let src: i64 = conn.query_row("SELECT id FROM symbols WHERE name='Src'", [], |r| r.get(0)).unwrap();
        let dst: i64 = conn.query_row("SELECT id FROM symbols WHERE name='Dst'", [], |r| r.get(0)).unwrap();

        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'type_ref', 1.0)",
            rusqlite::params![src, dst],
        ).unwrap();

        let items = incoming_calls(&db, "Dst", 0).unwrap();
        assert_eq!(items.len(), 1, "type_ref edges should appear as incoming calls (IDE-038)");
        assert_eq!(items[0].name, "Src");
    }

    #[test]
    fn structural_edges_excluded_from_call_hierarchy() {
        // 'inherits' and 'implements' are structural — they should NOT appear
        // in the call hierarchy (they are not usage edges).
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('d.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        for (name, qname) in [("Child", "NS.Child"), ("Parent", "NS.Parent")] {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'class', 1, 0)",
                rusqlite::params![fid, name, qname],
            ).unwrap();
        }
        let child:  i64 = conn.query_row("SELECT id FROM symbols WHERE name='Child'",  [], |r| r.get(0)).unwrap();
        let parent: i64 = conn.query_row("SELECT id FROM symbols WHERE name='Parent'", [], |r| r.get(0)).unwrap();

        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'inherits', 1.0)",
            rusqlite::params![child, parent],
        ).unwrap();

        let items = incoming_calls(&db, "Parent", 0).unwrap();
        assert!(items.is_empty(), "inherits edges should not appear in call hierarchy");
    }
}
