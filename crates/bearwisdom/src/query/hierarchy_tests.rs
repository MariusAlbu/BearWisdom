use super::*;
use crate::db::Database;

fn setup_two_package_repo(db: &Database) -> (i64, i64, i64, i64, i64, i64) {
    let conn = db.conn();

    conn.execute(
        "INSERT INTO packages (name, path, kind, is_service) VALUES ('pkg-a', 'packages/a', 'cargo', 0)",
        [],
    ).unwrap();
    let pkg_a = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO packages (name, path, kind, is_service) VALUES ('pkg-b', 'packages/b', 'cargo', 0)",
        [],
    ).unwrap();
    let pkg_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/a/lib.rs', 'h1', 'rust', 0, ?1)",
        rusqlite::params![pkg_a],
    ).unwrap();
    let file_a = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/b/lib.rs', 'h2', 'rust', 0, ?1)",
        rusqlite::params![pkg_b],
    ).unwrap();
    let file_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_a', 'a::fn_a', 'function', 1, 0)",
        rusqlite::params![file_a],
    ).unwrap();
    let sym_a = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_b', 'b::fn_b', 'function', 1, 0)",
        rusqlite::params![file_b],
    ).unwrap();
    let sym_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
        rusqlite::params![sym_a, sym_b],
    ).unwrap();

    (pkg_a, pkg_b, file_a, file_b, sym_a, sym_b)
}

// ---- packages level ----

#[test]
fn packages_level_returns_nodes_and_edge() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);

    let result = hierarchical_graph(&db, "packages", None, 500).unwrap();
    assert_eq!(result.level, "packages");
    assert_eq!(result.nodes.len(), 2);
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.edges[0].kind, "cross_package");
    assert_eq!(result.edges[0].weight, 1);
}

#[test]
fn packages_level_empty_for_single_project() {
    let db = Database::open_in_memory().unwrap();
    // No packages inserted.
    let result = hierarchical_graph(&db, "packages", None, 500).unwrap();
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
}

// ---- services level ----

#[test]
fn services_level_falls_back_to_all_packages_when_none_flagged() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);

    let result = hierarchical_graph(&db, "services", None, 500).unwrap();
    // No is_service=1 packages, so it falls back to all packages.
    assert_eq!(result.nodes.len(), 2);
    assert_eq!(result.level, "services");
}

#[test]
fn services_level_only_shows_service_packages_when_flagged() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);
    // Flag pkg-a as a service.
    db.conn()
        .execute(
            "UPDATE packages SET is_service = 1 WHERE path = 'packages/a'",
            [],
        )
        .unwrap();

    let result = hierarchical_graph(&db, "services", None, 500).unwrap();
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].kind, "service");
}

// ---- files level ----

#[test]
fn files_level_scoped_to_package() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);

    let result = hierarchical_graph(&db, "files", Some("packages/a"), 500).unwrap();
    assert_eq!(result.level, "files");
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].file_path.as_deref(), Some("packages/a/lib.rs"));
    // One cross-package file edge exists: packages/a/lib.rs → packages/b/lib.rs.
    // The files level shows outbound edges even when the target file is in another package.
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.edges[0].kind, "file_dependency");
    assert_eq!(result.edges[0].source, "file:packages/a/lib.rs");
    assert_eq!(result.edges[0].target, "file:packages/b/lib.rs");
}

#[test]
fn files_level_no_scope_returns_all_files() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);

    let result = hierarchical_graph(&db, "files", None, 500).unwrap();
    assert_eq!(result.nodes.len(), 2);
    // Cross-file edge exists: packages/a/lib.rs → packages/b/lib.rs.
    assert_eq!(result.edges.len(), 1);
}

// ---- symbols level ----

#[test]
fn symbols_level_returns_symbols_in_file() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);

    let result = hierarchical_graph(&db, "symbols", Some("packages/a/lib.rs"), 500).unwrap();
    assert_eq!(result.level, "symbols");
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].name, "fn_a");
    // Edge exists: fn_a calls fn_b (fn_b is not in this file but the edge is included).
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.edges[0].kind, "calls");
}

#[test]
fn symbols_level_empty_file_returns_empty() {
    let db = Database::open_in_memory().unwrap();
    let result = hierarchical_graph(&db, "symbols", Some("nonexistent.rs"), 500).unwrap();
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
}

// ---- breadcrumbs ----

#[test]
fn breadcrumbs_always_start_with_workspace() {
    let db = Database::open_in_memory().unwrap();
    for level in ["services", "packages", "files", "symbols"] {
        let result = hierarchical_graph(&db, level, None, 500).unwrap();
        assert_eq!(result.breadcrumbs[0].label, "Workspace");
    }
}

#[test]
fn files_breadcrumbs_include_package() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);
    let result = hierarchical_graph(&db, "files", Some("packages/a"), 500).unwrap();
    assert_eq!(result.breadcrumbs.len(), 2);
    assert_eq!(result.breadcrumbs[1].label, "a");
    assert_eq!(result.breadcrumbs[1].level, "files");
}

// ---- error handling ----

#[test]
fn unknown_level_returns_error() {
    let db = Database::open_in_memory().unwrap();
    let result = hierarchical_graph(&db, "bogus", None, 500);
    assert!(result.is_err());
}

// ---- node id format ----

#[test]
fn package_node_ids_have_pkg_prefix() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);
    let result = hierarchical_graph(&db, "packages", None, 500).unwrap();
    for node in &result.nodes {
        assert!(node.id.starts_with("pkg:"), "unexpected id: {}", node.id);
    }
}

#[test]
fn file_node_ids_have_file_prefix() {
    let db = Database::open_in_memory().unwrap();
    setup_two_package_repo(&db);
    let result = hierarchical_graph(&db, "files", None, 500).unwrap();
    for node in &result.nodes {
        assert!(node.id.starts_with("file:"), "unexpected id: {}", node.id);
    }
}
