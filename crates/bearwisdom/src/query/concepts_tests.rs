use super::*;
use crate::db::Database;

fn insert_symbol(db: &Database, path: &str, name: &str, qname: &str) -> i64 {
    let conn = &db.conn;
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
         ON CONFLICT(path) DO NOTHING",
        [path],
    ).unwrap();
    let fid: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).unwrap();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'class', 1, 0)",
        rusqlite::params![fid, name, qname],
    ).unwrap();
    conn.last_insert_rowid()
}

fn insert_concept(db: &Database, name: &str, pattern: Option<&str>) -> i64 {
    db.conn.execute(
        "INSERT INTO concepts (name, auto_pattern, created_at) VALUES (?1, ?2, 0)",
        rusqlite::params![name, pattern],
    ).unwrap();
    db.conn.last_insert_rowid()
}

#[test]
fn list_concepts_empty_database() {
    let db = Database::open_in_memory().unwrap();
    let concepts = list_concepts(&db).unwrap();
    assert!(concepts.is_empty());
}

#[test]
fn list_concepts_with_counts() {
    let db = Database::open_in_memory().unwrap();
    let cid = insert_concept(&db, "catalog", Some("App.Catalog.*"));
    let sid = insert_symbol(&db, "a.cs", "CatalogService", "App.Catalog.CatalogService");

    db.conn.execute(
        "INSERT INTO concept_members (concept_id, symbol_id, auto_assigned) VALUES (?1, ?2, 1)",
        rusqlite::params![cid, sid],
    ).unwrap();

    let concepts = list_concepts(&db).unwrap();
    assert_eq!(concepts.len(), 1);
    assert_eq!(concepts[0].name, "catalog");
    assert_eq!(concepts[0].member_count, 1);
}

#[test]
fn auto_assign_populates_members() {
    let db = Database::open_in_memory().unwrap();
    insert_concept(&db, "catalog", Some("App.Catalog.*"));
    insert_symbol(&db, "a.cs", "CatalogService", "App.Catalog.CatalogService");
    insert_symbol(&db, "b.cs", "OrderService",   "App.Orders.OrderService");

    let inserted = auto_assign_concepts(&db).unwrap();
    assert_eq!(inserted, 1, "Only CatalogService should be auto-assigned");

    let members = concept_members(&db, "catalog", 0).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].name, "CatalogService");
}

#[test]
fn auto_assign_is_idempotent() {
    let db = Database::open_in_memory().unwrap();
    insert_concept(&db, "catalog", Some("App.Catalog.*"));
    insert_symbol(&db, "a.cs", "CatalogService", "App.Catalog.CatalogService");

    let first  = auto_assign_concepts(&db).unwrap();
    let second = auto_assign_concepts(&db).unwrap();
    assert_eq!(first, 1);
    assert_eq!(second, 0, "Second run should insert 0 (already assigned)");
}

#[test]
fn concept_members_returns_empty_for_unknown_concept() {
    let db = Database::open_in_memory().unwrap();
    let members = concept_members(&db, "nonexistent", 0).unwrap();
    assert!(members.is_empty());
}

#[test]
fn discover_concepts_finds_namespace_prefixes() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(&db, "a.cs", "S1", "Microsoft.eShop.Catalog.CatalogService");
    insert_symbol(&db, "b.cs", "S2", "Microsoft.eShop.Orders.OrderService");
    insert_symbol(&db, "c.cs", "S3", "Top.Level.Something");

    let discovered = discover_concepts(&db).unwrap();
    assert!(discovered.contains(&"Microsoft.eShop".to_string()), "Should discover Microsoft.eShop");
    assert!(discovered.contains(&"Top.Level".to_string()), "Should discover Top.Level");
}

#[test]
fn discover_concepts_is_idempotent() {
    let db = Database::open_in_memory().unwrap();
    insert_symbol(&db, "a.cs", "S1", "App.Catalog.CatalogService");

    let first  = discover_concepts(&db).unwrap();
    let second = discover_concepts(&db).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 0, "Second discovery should not create duplicates");
}

#[test]
fn discover_concepts_ignores_short_names() {
    let db = Database::open_in_memory().unwrap();
    // "Foo.Bar" has only one dot — skip.
    insert_symbol(&db, "a.cs", "S1", "Foo.Bar");

    let discovered = discover_concepts(&db).unwrap();
    assert!(discovered.is_empty(), "Two-segment names should not create concepts");
}
