use super::*;
use crate::db::Database;

/// Minimal setup: one file, multiple symbols and edges.
fn setup_graph(db: &Database) -> (i64, i64, i64, i64) {
    let conn = &db.conn;
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('a.cs', 'h', 'csharp', 0)",
        [],
    ).unwrap();
    let fid = conn.last_insert_rowid();

    // Graph: D → C → B → A   (A is the center we'll query)
    for (name, qname, line) in [("A", "NS.A", 1i64), ("B", "NS.B", 2), ("C", "NS.C", 3), ("D", "NS.D", 4)] {
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'method', ?4, 0)",
            rusqlite::params![fid, name, qname, line],
        ).unwrap();
    }
    let a: i64 = conn.query_row("SELECT id FROM symbols WHERE name='A'", [], |r| r.get(0)).unwrap();
    let b: i64 = conn.query_row("SELECT id FROM symbols WHERE name='B'", [], |r| r.get(0)).unwrap();
    let c: i64 = conn.query_row("SELECT id FROM symbols WHERE name='C'", [], |r| r.get(0)).unwrap();
    let d: i64 = conn.query_row("SELECT id FROM symbols WHERE name='D'", [], |r| r.get(0)).unwrap();

    // B calls A, C calls B, D calls C
    conn.execute("INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)", rusqlite::params![b, a]).unwrap();
    conn.execute("INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)", rusqlite::params![c, b]).unwrap();
    conn.execute("INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)", rusqlite::params![d, c]).unwrap();

    (a, b, c, d)
}

#[test]
fn blast_radius_direct_callers_depth_1() {
    let db = Database::open_in_memory().unwrap();
    setup_graph(&db);

    let result = blast_radius(&db, "A", 1).unwrap().expect("symbol not found");
    assert_eq!(result.center.name, "A");
    // Only B directly calls A.
    assert_eq!(result.affected.len(), 1);
    assert_eq!(result.affected[0].name, "B");
    assert_eq!(result.affected[0].depth, 1);
}

#[test]
fn blast_radius_depth_2_includes_transitive() {
    let db = Database::open_in_memory().unwrap();
    setup_graph(&db);

    let result = blast_radius(&db, "A", 2).unwrap().expect("symbol not found");
    let names: Vec<&str> = result.affected.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"B"), "B should be at depth 1");
    assert!(names.contains(&"C"), "C should be at depth 2");
    assert!(!names.contains(&"D"), "D should not appear at depth <= 2");
}

#[test]
fn blast_radius_full_chain() {
    let db = Database::open_in_memory().unwrap();
    setup_graph(&db);

    let result = blast_radius(&db, "A", 10).unwrap().expect("symbol not found");
    let names: Vec<&str> = result.affected.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"B"));
    assert!(names.contains(&"C"));
    assert!(names.contains(&"D"));
    assert_eq!(result.total_affected, 3);
}

#[test]
fn blast_radius_returns_none_for_unknown_symbol() {
    let db = Database::open_in_memory().unwrap();
    let result = blast_radius(&db, "DoesNotExist", 3).unwrap();
    assert!(result.is_none());
}

#[test]
fn blast_radius_symbol_with_no_callers() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('b.cs', 'h', 'csharp', 0)",
        [],
    ).unwrap();
    let fid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'Lonely', 'NS.Lonely', 'class', 1, 0)",
        [fid],
    ).unwrap();

    let result = blast_radius(&db, "Lonely", 5).unwrap().expect("symbol should exist");
    assert_eq!(result.center.name, "Lonely");
    assert!(result.affected.is_empty());
    assert_eq!(result.total_affected, 0);
}
