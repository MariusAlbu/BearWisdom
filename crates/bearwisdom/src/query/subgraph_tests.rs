use super::*;
use crate::db::Database;

fn insert_symbol(db: &Database, path: &str, name: &str, qname: &str) -> i64 {
    let conn = db.conn();
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

fn insert_edge(db: &Database, src: i64, tgt: i64) {
    db.conn().execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
        rusqlite::params![src, tgt],
    ).unwrap();
}

#[test]
fn export_full_graph_includes_all_symbols_and_edges() {
    let db = Database::open_in_memory().unwrap();
    let s1 = insert_symbol(&db, "a.cs", "Foo", "App.Foo");
    let s2 = insert_symbol(&db, "b.cs", "Bar", "App.Bar");
    insert_edge(&db, s1, s2);

    let graph = export_graph(&db, None, 1000).unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
}

#[test]
fn export_with_prefix_filter_excludes_other_namespaces() {
    let db = Database::open_in_memory().unwrap();
    let s1 = insert_symbol(&db, "a.cs", "CatalogService", "App.Catalog.CatalogService");
    let s2 = insert_symbol(&db, "b.cs", "OrderService",   "App.Orders.OrderService");
    insert_edge(&db, s1, s2);

    let graph = export_graph(&db, Some("App.Catalog"), 1000).unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].name, "CatalogService");
    // The edge connects to OrderService which is excluded → no edges.
    assert_eq!(graph.edges.len(), 0);
}

#[test]
fn export_with_concept_filter() {
    let db = Database::open_in_memory().unwrap();
    let s1 = insert_symbol(&db, "a.cs", "AuthService", "App.Auth.AuthService");
    let _s2 = insert_symbol(&db, "b.cs", "Other",       "App.Other.Other");

    // Create concept and assign s1 to it.
    db.conn().execute(
        "INSERT INTO concepts (name, auto_pattern, created_at) VALUES ('auth', 'App.Auth.*', 0)",
        [],
    ).unwrap();
    let cid: i64 = db.conn().last_insert_rowid();
    db.conn().execute(
        "INSERT INTO concept_members (concept_id, symbol_id, auto_assigned) VALUES (?1, ?2, 1)",
        rusqlite::params![cid, s1],
    ).unwrap();

    let graph = export_graph(&db, Some("@auth"), 1000).unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].name, "AuthService");
}

#[test]
fn export_respects_max_nodes_cap() {
    let db = Database::open_in_memory().unwrap();
    let mut ids = Vec::new();
    for i in 0..20 {
        ids.push(insert_symbol(&db, "a.cs", &format!("Sym{i}"), &format!("App.Sym{i}")));
    }
    // Connect them so they pass the "has edges" filter.
    for i in 0..19 {
        insert_edge(&db, ids[i], ids[i + 1]);
    }

    let graph = export_graph(&db, None, 5).unwrap();
    assert_eq!(graph.nodes.len(), 5, "Should respect max_nodes cap");
}

#[test]
fn export_edges_only_between_included_nodes() {
    let db = Database::open_in_memory().unwrap();
    let s1 = insert_symbol(&db, "a.cs", "A", "NS.A");
    let s2 = insert_symbol(&db, "b.cs", "B", "NS.B");
    let s3 = insert_symbol(&db, "c.cs", "C", "Other.C");
    insert_edge(&db, s1, s2);
    insert_edge(&db, s1, s3); // s3 will be excluded by prefix filter

    let graph = export_graph(&db, Some("NS"), 1000).unwrap();
    assert_eq!(graph.nodes.len(), 2);
    // Only the edge between s1 and s2 should be included.
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].source_id, s1);
    assert_eq!(graph.edges[0].target_id, s2);
}

#[test]
fn export_graph_json_is_valid_json() {
    let db = Database::open_in_memory().unwrap();
    let s1 = insert_symbol(&db, "a.cs", "Foo", "App.Foo");
    let s2 = insert_symbol(&db, "b.cs", "Bar", "App.Bar");
    insert_edge(&db, s1, s2);

    let json = export_graph_json(&db, None, 100).unwrap();
    // Verify it parses without error and contains the expected keys.
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["nodes"].is_array());
    assert!(parsed["edges"].is_array());
}

#[test]
fn export_empty_graph() {
    let db = Database::open_in_memory().unwrap();
    let graph = export_graph(&db, None, 1000).unwrap();
    assert!(graph.nodes.is_empty());
    assert!(graph.edges.is_empty());
}
