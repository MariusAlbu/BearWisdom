use super::*;
use crate::db::Database;

/// Insert a file row and return its id.
fn insert_file(db: &Database, path: &str, lang: &str) -> i64 {
    db.conn().execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', ?2, 0)",
        rusqlite::params![path, lang],
    ).unwrap();
    db.conn().last_insert_rowid()
}

/// Insert a symbol row and return its id.
fn insert_symbol(db: &Database, file_id: i64, name: &str, qname: &str, kind: &str, vis: Option<&str>) -> i64 {
    db.conn().execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility)
         VALUES (?1, ?2, ?3, ?4, 1, 0, ?5)",
        rusqlite::params![file_id, name, qname, kind, vis],
    ).unwrap();
    db.conn().last_insert_rowid()
}

/// Insert a directed edge.
fn insert_edge(db: &Database, src: i64, tgt: i64, kind: &str) {
    db.conn().execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, ?3, 1.0)",
        rusqlite::params![src, tgt, kind],
    ).unwrap();
}

#[test]
fn overview_totals_are_correct() {
    let db = Database::open_in_memory().unwrap();
    let f1 = insert_file(&db, "a.cs", "csharp");
    let s1 = insert_symbol(&db, f1, "Foo", "App.Foo", "class", Some("public"));
    let s2 = insert_symbol(&db, f1, "Bar", "App.Bar", "class", Some("public"));
    insert_edge(&db, s1, s2, "calls");

    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.total_files, 1);
    assert_eq!(ov.total_symbols, 2);
    assert_eq!(ov.total_edges, 1);
}

#[test]
fn overview_language_stats() {
    let db = Database::open_in_memory().unwrap();
    let f1 = insert_file(&db, "a.cs", "csharp");
    let f2 = insert_file(&db, "b.ts", "typescript");
    insert_symbol(&db, f1, "Foo", "App.Foo", "class", None);
    insert_symbol(&db, f2, "bar", "bar", "function", None);

    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.languages.len(), 2);
    // Each language should have 1 file.
    assert!(ov.languages.iter().all(|l| l.file_count == 1));
}

#[test]
fn overview_hotspots_ranked_by_incoming() {
    let db = Database::open_in_memory().unwrap();
    let f = insert_file(&db, "a.cs", "csharp");
    let popular = insert_symbol(&db, f, "Hub", "App.Hub", "class", None);
    let s1 = insert_symbol(&db, f, "A", "App.A", "method", None);
    let s2 = insert_symbol(&db, f, "B", "App.B", "method", None);
    let s3 = insert_symbol(&db, f, "C", "App.C", "method", None);
    insert_edge(&db, s1, popular, "calls");
    insert_edge(&db, s2, popular, "calls");
    insert_edge(&db, s3, popular, "type_ref");

    let ov = get_overview(&db).unwrap();
    assert!(!ov.hotspots.is_empty());
    assert_eq!(ov.hotspots[0].name, "Hub");
    assert_eq!(ov.hotspots[0].incoming_refs, 3);
}

#[test]
fn overview_entry_points_filters_public() {
    let db = Database::open_in_memory().unwrap();
    let f = insert_file(&db, "a.cs", "csharp");
    insert_symbol(&db, f, "PubClass",  "App.PubClass",  "class", Some("public"));
    insert_symbol(&db, f, "PrivClass", "App.PrivClass", "class", Some("private"));

    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.entry_points.len(), 1);
    assert_eq!(ov.entry_points[0].name, "PubClass");
}

#[test]
fn overview_empty_database() {
    let db = Database::open_in_memory().unwrap();
    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.total_files, 0);
    assert_eq!(ov.total_symbols, 0);
    assert_eq!(ov.total_edges, 0);
    assert!(ov.languages.is_empty());
    assert!(ov.hotspots.is_empty());
    assert!(ov.entry_points.is_empty());
}
