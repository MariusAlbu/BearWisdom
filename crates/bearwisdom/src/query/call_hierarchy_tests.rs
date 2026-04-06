use super::*;
use crate::db::Database;

/// Build a small graph: Caller → Service, Service → Db.
fn setup(db: &Database) -> (i64, i64, i64) {
    let conn = db.conn();
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
    let conn = db.conn();
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
    let conn = db.conn();
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
    let conn = db.conn();
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
