use super::*;
use crate::db::Database;

fn insert_symbol(db: &Database, path: &str, name: &str, qname: &str, kind: &str, line: u32) -> i64 {
    let conn = db.conn();
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
