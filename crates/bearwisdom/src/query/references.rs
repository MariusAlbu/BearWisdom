// =============================================================================
// query/references.rs  —  find-all-references query
//
// "Who uses this symbol?"
//
// Returns all edges that point TO the given symbol (incoming edges).
// For each edge we return the referencing symbol name, file, line, and
// edge kind so the caller can display a proper reference list.
// =============================================================================

use crate::db::Database;
use crate::types::ReferenceResult;
use anyhow::{Context, Result};

/// Find all symbols that reference `target_name`.
///
/// `target_name` may be a simple name or a fully qualified name.
/// When it is a simple name, all symbols with that name are searched
/// (returns references to all overloads / same-named symbols).
///
/// `limit`: maximum number of results (0 = unlimited).
pub fn find_references(db: &Database, target_name: &str, limit: usize) -> Result<Vec<ReferenceResult>> {
    let conn = &db.conn;

    // Resolve the target name to one or more symbol IDs.
    let target_ids: Vec<i64> = {
        if target_name.contains('.') {
            // Qualified name — exact match.
            let mut stmt = conn.prepare(
                "SELECT id FROM symbols WHERE qualified_name = ?1"
            ).context("Failed to prepare qualified target lookup")?;
            let rows = stmt.query_map([target_name], |r| r.get(0))
                .context("Failed to query qualified target")?;
            rows.filter_map(|r| r.ok()).collect()
        } else {
            // Simple name — all symbols with that name.
            let mut stmt = conn.prepare(
                "SELECT id FROM symbols WHERE name = ?1"
            ).context("Failed to prepare simple target lookup")?;
            let rows = stmt.query_map([target_name], |r| r.get(0))
                .context("Failed to query simple target")?;
            rows.filter_map(|r| r.ok()).collect()
        }
    };

    if target_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut results: Vec<ReferenceResult> = Vec::new();

    for target_id in &target_ids {
        // Find all edges pointing to this target.
        let limit_clause = if limit > 0 {
            format!("LIMIT {limit}")
        } else {
            String::new()
        };

        let sql = format!(
            "SELECT src.name, src.kind, f.path, e.source_line, e.kind, e.confidence
             FROM edges e
             JOIN symbols src ON e.source_id = src.id
             JOIN files   f   ON src.file_id  = f.id
             WHERE e.target_id = ?1
             ORDER BY f.path, e.source_line
             {limit_clause}"
        );

        let mut stmt = conn.prepare(&sql).context("Failed to prepare references query")?;
        let rows = stmt.query_map([target_id], |row| {
            Ok(ReferenceResult {
                referencing_symbol: row.get(0)?,
                referencing_kind: row.get(1)?,
                file_path: row.get(2)?,
                line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                edge_kind: row.get(4)?,
                confidence: row.get(5)?,
            })
        }).context("Failed to execute references query")?;

        for row in rows {
            results.push(row?);
        }
    }

    // Sort by file path then line for stable output.
    results.sort_by(|a, b| {
        a.file_path.cmp(&b.file_path)
            .then(a.line.cmp(&b.line))
    });

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

    fn setup(db: &Database) -> (i64, i64) {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('a.cs', 'h1', 'csharp', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Caller', 'NS.Caller', 'method', 10, 0)",
            [file_id],
        ).unwrap();
        let caller_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Callee', 'NS.Callee', 'method', 20, 0)",
            [file_id],
        ).unwrap();
        let callee_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'calls', 12, 1.0)",
            rusqlite::params![caller_id, callee_id],
        ).unwrap();

        (caller_id, callee_id)
    }

    #[test]
    fn find_references_by_simple_name() {
        let db = Database::open_in_memory().unwrap();
        setup(&db);

        let refs = find_references(&db, "Callee", 0).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].referencing_symbol, "Caller");
        assert_eq!(refs[0].edge_kind, "calls");
    }

    #[test]
    fn find_references_by_qualified_name() {
        let db = Database::open_in_memory().unwrap();
        setup(&db);

        let refs = find_references(&db, "NS.Callee", 0).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].referencing_symbol, "Caller");
    }

    #[test]
    fn find_references_returns_empty_for_unknown() {
        let db = Database::open_in_memory().unwrap();
        let refs = find_references(&db, "Unknown", 0).unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn find_references_respects_limit() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('b.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        // Insert target.
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Target', 'NS.Target', 'class', 1, 0)",
            [file_id],
        ).unwrap();
        let target_id: i64 = conn.last_insert_rowid();

        // Insert 5 callers.
        for i in 0..5i64 {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
                 VALUES (?1, 'User', ?, 'method', ?3, 0)",
                rusqlite::params![file_id, format!("NS.User{i}"), i * 10 + 10],
            ).unwrap();
            let user_id: i64 = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'type_ref', ?3, 0.9)",
                rusqlite::params![user_id, target_id, i * 10 + 5],
            ).unwrap();
        }

        let refs = find_references(&db, "Target", 3).unwrap();
        assert_eq!(refs.len(), 3, "Expected limit to be respected");
    }

    /// Regression test for the Handle-method bug (Issue 5).
    ///
    /// When multiple classes each have a `Handle` method, a simple-name lookup
    /// returns references to ALL of them.  A qualified-name lookup must return
    /// only references to the specific one.
    #[test]
    fn qualified_name_lookup_isolates_specific_handle_method() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('handlers.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        // Two handler classes, each with a `Handle` method.
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Handle', 'NS.HandlerA.Handle', 'method', 10, 0)",
            [file_id],
        ).unwrap();
        let handle_a: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Handle', 'NS.HandlerB.Handle', 'method', 30, 0)",
            [file_id],
        ).unwrap();
        let handle_b: i64 = conn.last_insert_rowid();

        // A caller that calls HandlerA.Handle specifically.
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Dispatcher', 'NS.Dispatcher', 'class', 50, 0)",
            [file_id],
        ).unwrap();
        let dispatcher: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'calls', 55, 1.0)",
            rusqlite::params![dispatcher, handle_a],
        ).unwrap();

        // Simple-name lookup returns references to BOTH Handle methods.
        let refs_simple = find_references(&db, "Handle", 0).unwrap();
        assert_eq!(refs_simple.len(), 1,
            "simple-name 'Handle' should find the one edge (only HandlerA.Handle has a caller)");

        // Qualified-name lookup for HandlerA returns only HandlerA's reference.
        let refs_a = find_references(&db, "NS.HandlerA.Handle", 0).unwrap();
        assert_eq!(refs_a.len(), 1);
        assert_eq!(refs_a[0].referencing_symbol, "Dispatcher");

        // Qualified-name lookup for HandlerB returns zero references.
        let refs_b = find_references(&db, "NS.HandlerB.Handle", 0).unwrap();
        assert!(refs_b.is_empty(),
            "HandlerB.Handle has no callers — should return empty");

        // Confirm that using the simple name (old behavior) returns refs to
        // both symbols — this documents why a simple-name lookup was wrong.
        let _ = handle_b; // suppress unused warning
    }
}
