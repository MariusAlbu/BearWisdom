use super::*;
use rusqlite::Connection;

fn make_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn, true).unwrap();
    create_schema(&conn).unwrap();
    conn
}

#[test]
fn schema_creates_all_tables() {
    let conn = make_db();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    for expected in &[
        "files", "symbols", "edges", "unresolved_refs", "external_refs", "imports",
        "routes", "db_mappings", "annotations", "concepts", "concept_members",
        "lsp_edge_meta", "code_chunks", "flow_edges", "search_history",
    ] {
        assert!(
            tables.contains(&expected.to_string()),
            "Missing table: {expected}. Found: {tables:?}"
        );
    }
}

#[test]
fn schema_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn, true).unwrap();
    // Apply twice — should not error.
    create_schema(&conn).unwrap();
    create_schema(&conn).unwrap();
}

#[test]
fn cascade_delete_removes_symbols_when_file_deleted() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('a.cs', 'h1', 'csharp', 0)",
        [],
    ).unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'Foo', 'NS.Foo', 'class', 1, 0)",
        [file_id],
    ).unwrap();

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);

    conn.execute("DELETE FROM files WHERE id = ?1", [file_id]).unwrap();

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0, "Symbols should cascade-delete with file");
}

#[test]
fn fts5_trigger_indexes_new_symbols() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('x.cs', 'h', 'csharp', 0)",
        [],
    ).unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    // Insert a symbol — the symbols_ai trigger should add it to FTS.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'MyService', 'App.MyService', 'class', 1, 0)",
        [file_id],
    ).unwrap();

    // FTS5 MATCH query should find it.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH 'MyService'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "FTS5 trigger should have indexed the symbol");
}

#[test]
fn fts5_trigger_removes_deleted_symbols() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('x.cs', 'h', 'csharp', 0)",
        [],
    ).unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'DeleteMe', 'App.DeleteMe', 'class', 1, 0)",
        [file_id],
    ).unwrap();
    let sym_id: i64 = conn.last_insert_rowid();

    // Confirm it is findable.
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH 'DeleteMe'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1);

    // Delete the symbol — the symbols_ad trigger should remove from FTS.
    conn.execute("DELETE FROM symbols WHERE id = ?1", [sym_id]).unwrap();

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH 'DeleteMe'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 0, "FTS5 trigger should have removed the deleted symbol");
}

#[test]
fn schema_creates_all_indexes() {
    let conn = make_db();

    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type='index' AND name LIKE 'idx_%' \
             ORDER BY name",
        )
        .unwrap();

    let indexes: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // Verify covering indexes exist (these replaced the old single-column variants).
    assert!(
        indexes.contains(&"idx_unresolved_source_cov".to_string()),
        "Missing idx_unresolved_source_cov. Found: {indexes:?}"
    );
    assert!(
        indexes.contains(&"idx_flow_edges_type".to_string()),
        "Missing idx_flow_edges_type. Found: {indexes:?}"
    );

    // Sample of other critical indexes.
    assert!(indexes.contains(&"idx_symbols_name".to_string()));
    assert!(indexes.contains(&"idx_symbols_qualified".to_string()));
    assert!(indexes.contains(&"idx_edges_source_cov".to_string()));
    assert!(indexes.contains(&"idx_edges_target_cov".to_string()));
    assert!(indexes.contains(&"idx_flow_source".to_string()));
    assert!(indexes.contains(&"idx_flow_target".to_string()));

    // Sanity: at least 25 indexes must exist.
    assert!(
        indexes.len() >= 25,
        "Expected >= 25 indexes, found {}",
        indexes.len()
    );
}

#[test]
fn unique_edge_constraint_prevents_duplicates() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('a.cs', 'h1', 'csharp', 0)",
        [],
    ).unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    for (name, qname) in [("Foo", "NS.Foo"), ("Bar", "NS.Bar")] {
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, 'class', 1, 0)",
            rusqlite::params![file_id, name, qname],
        ).unwrap();
    }

    let src: i64 = conn.query_row("SELECT id FROM symbols WHERE name='Foo'", [], |r| r.get(0)).unwrap();
    let tgt: i64 = conn.query_row("SELECT id FROM symbols WHERE name='Bar'", [], |r| r.get(0)).unwrap();

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 5, 1.0)",
        rusqlite::params![src, tgt],
    ).unwrap();

    let result = conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 5, 1.0)",
        rusqlite::params![src, tgt],
    );
    assert!(result.is_err(), "Duplicate edge should fail UNIQUE constraint");
}
