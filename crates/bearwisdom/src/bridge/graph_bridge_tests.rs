use super::*;
use crate::db::Database;

fn make_bridge() -> GraphBridge {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let lsp = Arc::new(LspManager::new("/tmp/test-workspace"));
    GraphBridge::new(db, lsp, "/tmp/test-workspace")
}

/// Insert a file + two symbols into the DB; return (file_id, src_id, tgt_id).
fn seed_symbols(bridge: &GraphBridge) -> (i64, i64, i64) {
    let guard = bridge.db.lock().unwrap();

    guard
        .conn
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/foo.rs', 'abc', 'rust', 0)",
            [],
        )
        .unwrap();
    let file_id = guard.conn.last_insert_rowid();

    guard
        .conn
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
             VALUES (?1, 'foo', 'mod::foo', 'function', 1, 0, 10)",
            [file_id],
        )
        .unwrap();
    let src_id = guard.conn.last_insert_rowid();

    guard
        .conn
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
             VALUES (?1, 'bar', 'mod::bar', 'function', 20, 0, 30)",
            [file_id],
        )
        .unwrap();
    let tgt_id = guard.conn.last_insert_rowid();

    (file_id, src_id, tgt_id)
}

#[test]
fn test_persist_lsp_edge_inserts() {
    let bridge = make_bridge();
    let (_, src_id, tgt_id) = seed_symbols(&bridge);

    let written = bridge
        .persist_lsp_edge(src_id, tgt_id, "calls", Some(5), "test-server")
        .unwrap();

    assert!(written, "first write should return true");

    // Verify the edge exists with confidence 1.0.
    let guard = bridge.db.lock().unwrap();
    let conf: f64 = guard
        .conn
        .query_row(
            "SELECT confidence FROM edges WHERE source_id = ?1 AND target_id = ?2",
            [src_id, tgt_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 1.0).abs() < f64::EPSILON);

    // Verify lsp_edge_meta was written.
    let meta_count: i64 = guard
        .conn
        .query_row("SELECT COUNT(*) FROM lsp_edge_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(meta_count, 1);
}

#[test]
fn test_persist_lsp_edge_upgrades() {
    let bridge = make_bridge();
    let (_, src_id, tgt_id) = seed_symbols(&bridge);

    // Insert an edge at 0.5 confidence first.
    {
        let guard = bridge.db.lock().unwrap();
        guard
            .conn
            .execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'calls', 5, 0.5)",
                [src_id, tgt_id],
            )
            .unwrap();
    }

    bridge
        .persist_lsp_edge(src_id, tgt_id, "calls", Some(5), "test-server")
        .unwrap();

    let guard = bridge.db.lock().unwrap();
    let conf: f64 = guard
        .conn
        .query_row(
            "SELECT confidence FROM edges WHERE source_id = ?1 AND target_id = ?2",
            [src_id, tgt_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 1.0).abs() < f64::EPSILON, "confidence should be upgraded to 1.0");
}

#[test]
fn test_upgrade_confidence() {
    let bridge = make_bridge();
    let (_, src_id, tgt_id) = seed_symbols(&bridge);

    // Insert at 0.5.
    {
        let guard = bridge.db.lock().unwrap();
        guard
            .conn
            .execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'calls', NULL, 0.5)",
                [src_id, tgt_id],
            )
            .unwrap();
    }

    let upgraded = bridge
        .upgrade_confidence(src_id, tgt_id, "calls", 0.9)
        .unwrap();
    assert!(upgraded);

    let guard = bridge.db.lock().unwrap();
    let conf: f64 = guard
        .conn
        .query_row(
            "SELECT confidence FROM edges WHERE source_id = ?1 AND target_id = ?2",
            [src_id, tgt_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 0.9).abs() < f64::EPSILON);

    // Upgrading to a lower value should be a no-op.
    drop(guard);
    let downgrade = bridge.upgrade_confidence(src_id, tgt_id, "calls", 0.3).unwrap();
    assert!(!downgrade);
}

#[test]
fn test_invalidate_file_edges() {
    let bridge = make_bridge();
    let (_, src_id, tgt_id) = seed_symbols(&bridge);

    // Write an LSP edge.
    bridge
        .persist_lsp_edge(src_id, tgt_id, "calls", None, "server")
        .unwrap();

    // Sanity: meta should exist.
    assert_eq!(bridge.lsp_edge_count().unwrap(), 1);

    let invalidated = bridge.invalidate_file_edges("src/foo.rs").unwrap();
    assert_eq!(invalidated, 1);

    // Meta should be gone.
    assert_eq!(bridge.lsp_edge_count().unwrap(), 0);

    // Confidence should be reset to 0.50.
    let guard = bridge.db.lock().unwrap();
    let conf: f64 = guard
        .conn
        .query_row(
            "SELECT confidence FROM edges WHERE source_id = ?1 AND target_id = ?2",
            [src_id, tgt_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 0.50).abs() < f64::EPSILON);
}

#[test]
fn test_location_to_symbol_id() {
    let bridge = make_bridge();
    let (_, src_id, _) = seed_symbols(&bridge);

    // Symbol 'foo' lives on lines 1–10 in src/foo.rs.
    // The workspace root is /tmp/test-workspace so the URI would be:
    //   file:///tmp/test-workspace/src/foo.rs
    let uri = "file:///tmp/test-workspace/src/foo.rs";
    let found = bridge.location_to_symbol_id(uri, 5, 0).unwrap();
    assert_eq!(found, Some(src_id));

    // Line 100 is outside any symbol.
    let not_found = bridge.location_to_symbol_id(uri, 100, 0).unwrap();
    assert!(not_found.is_none());
}

#[test]
fn test_find_target_column_found() {
    let dir = tempfile::TempDir::new().unwrap();
    // Line 0: "let foo = bar + baz;"
    // Line 1: "const greet = hello();"
    std::fs::write(dir.path().join("src.ts"), "let foo = bar + baz;\nconst greet = hello();\n")
        .unwrap();
    // "bar" starts at byte offset 10 on line 0
    let col = GraphBridge::find_target_column(dir.path(), "src.ts", 0, "bar");
    assert_eq!(col, 10);
    // "greet" starts at byte offset 6 on line 1
    let col2 = GraphBridge::find_target_column(dir.path(), "src.ts", 1, "greet");
    assert_eq!(col2, 6);
}

#[test]
fn test_find_target_column_not_found_returns_zero() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("src.ts"), "let x = 1;\n").unwrap();
    // "missing" is not on line 0
    let col = GraphBridge::find_target_column(dir.path(), "src.ts", 0, "missing");
    assert_eq!(col, 0);
}

#[test]
fn test_find_target_column_missing_file_returns_zero() {
    let dir = tempfile::TempDir::new().unwrap();
    let col = GraphBridge::find_target_column(dir.path(), "nonexistent.ts", 0, "foo");
    assert_eq!(col, 0);
}

#[test]
fn test_find_target_column_line_out_of_range_returns_zero() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("src.ts"), "let x = 1;\n").unwrap();
    let col = GraphBridge::find_target_column(dir.path(), "src.ts", 999, "x");
    assert_eq!(col, 0);
}

#[test]
fn test_uri_to_relative_path() {
    // Workspace root: /tmp/test-workspace
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let lsp = Arc::new(LspManager::new("/tmp/test-workspace"));
    let bridge = GraphBridge::new(db, lsp, "/tmp/test-workspace");

    #[cfg(not(target_os = "windows"))]
    {
        let rel = bridge.uri_to_relative_path("file:///tmp/test-workspace/src/main.rs");
        assert_eq!(rel, Some("src/main.rs".to_string()));

        // URI outside the workspace should return None (strip_prefix fails).
        let outside = bridge.uri_to_relative_path("file:///other/path/main.rs");
        assert!(outside.is_none());

        // Non-file URI.
        let none = bridge.uri_to_relative_path("https://example.com/foo");
        assert!(none.is_none());
    }

    // Sanity check: the function accepts well-formed file URIs.
    let some_uri = format!("file:///tmp/test-workspace/lib.rs");
    assert!(bridge.uri_to_relative_path(&some_uri).is_some());
}
