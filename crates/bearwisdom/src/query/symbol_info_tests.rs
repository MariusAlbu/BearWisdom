use super::*;
use crate::db::Database;

fn insert_symbol_full(
    db: &Database,
    path: &str,
    name: &str,
    qname: &str,
    kind: &str,
    scope_path: Option<&str>,
    sig: Option<&str>,
    line: u32,
    end_line: u32,
) -> i64 {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
         ON CONFLICT(path) DO NOTHING",
        [path],
    ).unwrap();
    let fid: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).unwrap();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, end_line, col, scope_path, signature, visibility)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, 'public')",
        rusqlite::params![fid, name, qname, kind, line, end_line, scope_path, sig],
    ).unwrap();
    conn.last_insert_rowid()
}

#[test]
fn symbol_info_basic_lookup() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol_full(&db, "a.cs", "FooService", "App.FooService", "class", None, None, 1, 50);

    let details = symbol_info(&db, "FooService", &crate::query::QueryOptions::full()).unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].name, "FooService");
    assert_eq!(details[0].start_line, 1);
    assert_eq!(details[0].end_line, 50);
    assert_eq!(details[0].kind, "class");
}

#[test]
fn symbol_info_by_qualified_name() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol_full(&db, "a.cs", "GetById", "App.FooService.GetById", "method", Some("App.FooService"), Some("Task<Foo> GetById(int id)"), 10, 20);

    let details = symbol_info(&db, "App.FooService.GetById", &crate::query::QueryOptions::full()).unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].qualified_name, "App.FooService.GetById");
    assert_eq!(details[0].signature.as_deref(), Some("Task<Foo> GetById(int id)"));
}

#[test]
fn symbol_info_edge_counts() {
    let db = Database::open_in_memory().unwrap();
    let s1 = insert_symbol_full(&db, "a.cs", "Caller", "App.Caller", "method", None, None, 1, 5);
    let s2 = insert_symbol_full(&db, "a.cs", "Callee", "App.Callee", "method", None, None, 10, 15);

    db.conn().execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
        rusqlite::params![s1, s2],
    ).unwrap();

    // Caller: 0 incoming, 1 outgoing.
    let caller_info = symbol_info(&db, "Caller", &crate::query::QueryOptions::full()).unwrap();
    assert_eq!(caller_info[0].incoming_edge_count, 0);
    assert_eq!(caller_info[0].outgoing_edge_count, 1);

    // Callee: 1 incoming, 0 outgoing.
    let callee_info = symbol_info(&db, "Callee", &crate::query::QueryOptions::full()).unwrap();
    assert_eq!(callee_info[0].incoming_edge_count, 1);
    assert_eq!(callee_info[0].outgoing_edge_count, 0);
}

#[test]
fn symbol_info_children() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol_full(&db, "a.cs", "MyClass", "App.MyClass", "class", None, None, 1, 100);
    insert_symbol_full(&db, "a.cs", "DoWork", "App.MyClass.DoWork", "method", Some("App.MyClass"), None, 10, 20);
    insert_symbol_full(&db, "a.cs", "Helper", "App.MyClass.Helper", "method", Some("App.MyClass"), None, 25, 35);

    let info = symbol_info(&db, "MyClass", &crate::query::QueryOptions::full()).unwrap();
    assert_eq!(info[0].children.len(), 2);
    let child_names: Vec<&str> = info[0].children.iter().map(|c| c.name.as_str()).collect();
    assert!(child_names.contains(&"DoWork"));
    assert!(child_names.contains(&"Helper"));
}

#[test]
fn symbol_info_returns_empty_for_unknown() {
    let db = Database::open_in_memory().unwrap();
    let result = symbol_info(&db, "NoSuchSymbol", &crate::query::QueryOptions::full()).unwrap();
    assert!(result.is_empty());
}
