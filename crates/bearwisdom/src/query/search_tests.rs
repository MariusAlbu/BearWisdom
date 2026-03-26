use super::*;
use crate::db::Database;

/// Insert a symbol and let the triggers populate symbols_fts.
fn insert_symbol(
    db: &Database,
    path: &str,
    name: &str,
    qname: &str,
    kind: &str,
    sig: Option<&str>,
    doc: Option<&str>,
) -> i64 {
    let conn = &db.conn;
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
         ON CONFLICT(path) DO NOTHING",
        [path],
    ).unwrap();
    let fid: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).unwrap();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, signature, doc_comment)
         VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?6)",
        rusqlite::params![fid, name, qname, kind, sig, doc],
    ).unwrap();
    conn.last_insert_rowid()
}

#[test]
fn search_finds_symbol_by_name() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(&db, "a.cs", "CatalogService", "App.CatalogService", "class", None, None);

    let results = search_symbols(&db, "CatalogService", 10).unwrap();
    assert!(!results.is_empty(), "Should find CatalogService");
    assert_eq!(results[0].name, "CatalogService");
}

#[test]
fn search_prefix_match() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(&db, "a.cs", "CatalogService", "App.CatalogService", "class", None, None);
    insert_symbol(&db, "b.cs", "CatalogItem",    "App.CatalogItem",    "class", None, None);
    insert_symbol(&db, "c.cs", "OrderService",   "App.OrderService",   "class", None, None);

    // Prefix query: "Catalog*" should match CatalogService and CatalogItem.
    let results = search_symbols(&db, "Catalog*", 10).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"CatalogService"), "Should match CatalogService");
    assert!(names.contains(&"CatalogItem"),    "Should match CatalogItem");
    assert!(!names.contains(&"OrderService"),  "Should not match OrderService");
}

#[test]
fn search_matches_in_doc_comment() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db, "a.cs", "GetItems", "App.GetItems", "method",
        None, Some("Returns all items from the authentication store"),
    );

    let results = search_symbols(&db, "authentication", 10).unwrap();
    assert!(!results.is_empty(), "Should find symbol via doc comment");
    assert_eq!(results[0].name, "GetItems");
}

#[test]
fn search_returns_empty_for_nonexistent_term() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(&db, "a.cs", "FooService", "App.FooService", "class", None, None);

    let results = search_symbols(&db, "ZzzNotFoundXxx", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_respects_limit() {
    let db = Database::open_in_memory().unwrap();
    for i in 0..10 {
        insert_symbol(
            &db, "a.cs",
            &format!("Widget{i}"), &format!("App.Widget{i}"),
            "class", None, None,
        );
    }

    let results = search_symbols(&db, "Widget*", 3).unwrap();
    assert!(results.len() <= 3, "Should respect limit of 3");
}

#[test]
fn search_empty_query_returns_empty() {
    let db = Database::open_in_memory().unwrap();
    let results = search_symbols(&db, "", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_matches_in_signature() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(
        &db, "a.cs", "Fetch", "App.Fetch", "method",
        Some("Task<CatalogItem> Fetch(int id)"), None,
    );

    let results = search_symbols(&db, "CatalogItem", 10).unwrap();
    // The FTS index includes the signature, so "CatalogItem" in the sig should match.
    assert!(!results.is_empty(), "Should match via signature");
}
