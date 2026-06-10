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
        "files",
        "symbols",
        "edges",
        "unresolved_refs",
        "external_refs",
        "imports",
        "routes",
        "db_mappings",
        "annotations",
        "concepts",
        "concept_members",
        "lsp_edge_meta",
        "code_chunks",
        "flow_edges",
        "search_history",
        "entry_points",
        "reachability",
        "package_resolution_health",
    ] {
        assert!(
            tables.contains(&expected.to_string()),
            "Missing table: {expected}. Found: {tables:?}"
        );
    }
}

#[test]
fn packages_has_is_publishable_default_true() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('pkg', 'crates/pkg', 'cargo')",
        [],
    )
    .unwrap();
    let v: i64 = conn
        .query_row(
            "SELECT is_publishable FROM packages WHERE name = 'pkg'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        v, 1,
        "is_publishable must default to 1 (publishable) for back-compat"
    );
}

#[test]
fn entry_points_composite_pk_allows_multi_contributor_rooting() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) \
         VALUES ('src/main.rs', 'h', 'rust', 0)",
        [],
    )
    .unwrap();
    let file_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'handler', 'app::handler', 'function', 10, 0)",
        [file_id],
    )
    .unwrap();
    let sym = conn.last_insert_rowid();

    // Same symbol rooted by two different contributors with different kinds.
    conn.execute(
        "INSERT INTO entry_points (symbol_id, kind, source, confidence) \
         VALUES (?1, 'route', 'axum-connector', 0.9)",
        [sym],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entry_points (symbol_id, kind, source, confidence) \
         VALUES (?1, 'user', 'user-roots-toml', 1.0)",
        [sym],
    )
    .unwrap();

    // Duplicate (symbol_id, kind, source) must fail.
    let dup = conn.execute(
        "INSERT INTO entry_points (symbol_id, kind, source) \
         VALUES (?1, 'route', 'axum-connector')",
        [sym],
    );
    assert!(dup.is_err(), "(symbol_id, kind, source) must be unique");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entry_points WHERE symbol_id = ?1",
            [sym],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn entry_points_cascade_when_symbol_deleted() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) \
         VALUES ('lib.rs', 'h', 'rust', 0)",
        [],
    )
    .unwrap();
    let file_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'main', 'main', 'function', 1, 0)",
        [file_id],
    )
    .unwrap();
    let sym = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO entry_points (symbol_id, kind, source) VALUES (?1, 'main', 'rust-plugin')",
        [sym],
    )
    .unwrap();

    conn.execute("DELETE FROM symbols WHERE id = ?1", [sym])
        .unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entry_points", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "entry_points must cascade when symbol is deleted");
}

#[test]
fn reachability_round_trip() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) \
         VALUES ('lib.rs', 'h', 'rust', 0)",
        [],
    )
    .unwrap();
    let file_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'foo', 'app::foo', 'function', 1, 0)",
        [file_id],
    )
    .unwrap();
    let sym = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO reachability (symbol_id, min_distance, path_confidence, via_kind) \
         VALUES (?1, 3, 0.7, 'dispatch_candidate')",
        [sym],
    )
    .unwrap();

    let (dist, conf, via): (i64, f64, Option<String>) = conn
        .query_row(
            "SELECT min_distance, path_confidence, via_kind FROM reachability WHERE symbol_id = ?1",
            [sym],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(dist, 3);
    assert!((conf - 0.7).abs() < 1e-9);
    assert_eq!(via.as_deref(), Some("dispatch_candidate"));

    // Symbol delete must cascade.
    conn.execute("DELETE FROM symbols WHERE id = ?1", [sym])
        .unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM reachability", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn package_resolution_health_round_trip() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('p', 'crates/p', 'cargo')",
        [],
    )
    .unwrap();
    let pkg = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO package_resolution_health \
         (package_id, resolution_rate, resolved_refs, unresolved_refs, low_conf_edges, trust_tier) \
         VALUES (?1, 97.5, 9750, 250, 100, 'review')",
        [pkg],
    )
    .unwrap();

    let (rate, tier): (f64, String) = conn
        .query_row(
            "SELECT resolution_rate, trust_tier FROM package_resolution_health WHERE package_id = ?1",
            [pkg],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((rate - 97.5).abs() < 1e-9);
    assert_eq!(tier, "review");

    conn.execute("DELETE FROM packages WHERE id = ?1", [pkg])
        .unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM package_resolution_health", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        count, 0,
        "package_resolution_health must cascade on package delete"
    );
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
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'Foo', 'NS.Foo', 'class', 1, 0)",
        [file_id],
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    conn.execute("DELETE FROM files WHERE id = ?1", [file_id])
        .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "Symbols should cascade-delete with file");
}

#[test]
fn fts5_trigger_indexes_new_symbols() {
    let conn = make_db();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('x.cs', 'h', 'csharp', 0)",
        [],
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    // Insert a symbol — the symbols_ai trigger should add it to FTS.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'MyService', 'App.MyService', 'class', 1, 0)",
        [file_id],
    )
    .unwrap();

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
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'DeleteMe', 'App.DeleteMe', 'class', 1, 0)",
        [file_id],
    )
    .unwrap();
    let sym_id: i64 = conn.last_insert_rowid();

    // Confirm it is findable.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH 'DeleteMe'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // Delete the symbol — the symbols_ad trigger should remove from FTS.
    conn.execute("DELETE FROM symbols WHERE id = ?1", [sym_id])
        .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH 'DeleteMe'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "FTS5 trigger should have removed the deleted symbol"
    );
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
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    for (name, qname) in [("Foo", "NS.Foo"), ("Bar", "NS.Bar")] {
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, 'class', 1, 0)",
            rusqlite::params![file_id, name, qname],
        )
        .unwrap();
    }

    let src: i64 = conn
        .query_row("SELECT id FROM symbols WHERE name='Foo'", [], |r| r.get(0))
        .unwrap();
    let tgt: i64 = conn
        .query_row("SELECT id FROM symbols WHERE name='Bar'", [], |r| r.get(0))
        .unwrap();

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 5, 1.0)",
        rusqlite::params![src, tgt],
    ).unwrap();

    let result = conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 5, 1.0)",
        rusqlite::params![src, tgt],
    );
    assert!(
        result.is_err(),
        "Duplicate edge should fail UNIQUE constraint"
    );
}
