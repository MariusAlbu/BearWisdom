use super::*;
use crate::db::Database;

fn setup_two_packages(db: &Database) {
    let conn = db.conn();

    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('pkg-a', 'packages/a', 'cargo')",
        [],
    )
    .unwrap();
    let pkg_a = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('pkg-b', 'packages/b', 'cargo')",
        [],
    )
    .unwrap();
    let pkg_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) \
         VALUES ('packages/a/lib.rs', 'h1', 'rust', 0, ?1)",
        rusqlite::params![pkg_a],
    )
    .unwrap();
    let file_a = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) \
         VALUES ('packages/b/lib.rs', 'h2', 'rust', 0, ?1)",
        rusqlite::params![pkg_b],
    )
    .unwrap();
    let file_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'fn_a', 'a::fn_a', 'function', 1, 0)",
        rusqlite::params![file_a],
    )
    .unwrap();
    let sym_a = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'fn_b', 'b::fn_b', 'function', 1, 0)",
        rusqlite::params![file_b],
    )
    .unwrap();
    let sym_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) \
         VALUES (?1, ?2, 'calls', 1.0)",
        rusqlite::params![sym_a, sym_b],
    )
    .unwrap();
}

#[test]
fn list_packages_returns_both_packages() {
    let db = Database::open_in_memory().unwrap();
    setup_two_packages(&db);

    let pkgs = list_packages(&db).unwrap();
    assert_eq!(pkgs.len(), 2);

    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"pkg-a"));
    assert!(names.contains(&"pkg-b"));
}

#[test]
fn list_packages_empty_for_single_project() {
    let db = Database::open_in_memory().unwrap();
    let pkgs = list_packages(&db).unwrap();
    assert!(pkgs.is_empty());
}

#[test]
fn workspace_overview_counts_cross_package_edge() {
    let db = Database::open_in_memory().unwrap();
    setup_two_packages(&db);

    let overview = workspace_overview(&db).unwrap();
    assert_eq!(overview.total_cross_package_edges, 1);
    assert_eq!(overview.packages.len(), 2);
}

#[test]
fn workspace_overview_empty_for_single_project() {
    let db = Database::open_in_memory().unwrap();
    let overview = workspace_overview(&db).unwrap();
    assert_eq!(overview.total_cross_package_edges, 0);
    assert!(overview.packages.is_empty());
    assert!(overview.shared_hotspots.is_empty());
}

#[test]
fn package_dependencies_detects_direction() {
    let db = Database::open_in_memory().unwrap();
    setup_two_packages(&db);

    let deps = package_dependencies(&db).unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].source_package, "pkg-a");
    assert_eq!(deps[0].target_package, "pkg-b");
    assert_eq!(deps[0].source_package_path, "packages/a");
    assert_eq!(deps[0].target_package_path, "packages/b");
    assert_eq!(deps[0].source_package_kind.as_deref(), Some("cargo"));
    assert_eq!(deps[0].edge_count, 1);
}

#[test]
fn package_dependencies_empty_for_single_project() {
    let db = Database::open_in_memory().unwrap();
    let deps = package_dependencies(&db).unwrap();
    assert!(deps.is_empty());
}

fn setup_graph_fixture(db: &Database) {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('pkg-a', 'packages/a', 'npm', '@myorg/a')",
        [],
    )
    .unwrap();
    let pkg_a = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('pkg-b', 'packages/b', 'npm', '@myorg/b')",
        [],
    )
    .unwrap();
    let pkg_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) \
         VALUES ('packages/a/src/a.ts', 'h1', 'typescript', 0, ?1)",
        rusqlite::params![pkg_a],
    )
    .unwrap();
    let file_a = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) \
         VALUES ('packages/b/src/b.ts', 'h2', 'typescript', 0, ?1)",
        rusqlite::params![pkg_b],
    )
    .unwrap();
    let file_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'fn_a', 'a.fn_a', 'function', 1, 0)",
        rusqlite::params![file_a],
    )
    .unwrap();
    let sym_a = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'fn_b', 'b.fn_b', 'function', 1, 0)",
        rusqlite::params![file_b],
    )
    .unwrap();
    let sym_b = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) \
         VALUES (?1, ?2, 'calls', 1, 1.0)",
        rusqlite::params![sym_a, sym_b],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) \
         VALUES (?1, ?2, 'calls', 2, 1.0)",
        rusqlite::params![sym_a, sym_b],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) \
         VALUES (?1, ?2, 'http_call', 3, 0.9)",
        rusqlite::params![sym_a, sym_b],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind) \
         VALUES (?1, 'typescript', '@myorg/b', '^1.0.0', 'runtime')",
        rusqlite::params![pkg_a],
    )
    .unwrap();
}

#[test]
fn workspace_graph_aggregates_code_flow_and_declared() {
    let db = Database::open_in_memory().unwrap();
    setup_graph_fixture(&db);

    let edges = workspace_graph(&db).unwrap();
    assert_eq!(edges.len(), 1);
    let e = &edges[0];
    assert_eq!(e.source_package, "pkg-a");
    assert_eq!(e.target_package, "pkg-b");
    assert_eq!(e.code_edges, 2);
    assert_eq!(e.flow_edges, 1);
    assert_eq!(e.total_edges, 3);
    assert!(e.declared_dep);
    assert_eq!(e.code_by_kind, vec![("calls".to_string(), 2)]);
    assert_eq!(e.flow_by_kind, vec![("http_call".to_string(), 1)]);
}

#[test]
fn workspace_graph_surfaces_declared_dep_with_zero_edges() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('x', 'x', 'npm', '@acme/x')",
        [],
    )
    .unwrap();
    let x = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('y', 'y', 'npm', '@acme/y')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind) \
         VALUES (?1, 'typescript', '@acme/y', '1', 'runtime')",
        rusqlite::params![x],
    )
    .unwrap();

    let edges = workspace_graph(&db).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(edges[0].declared_dep);
    assert_eq!(edges[0].total_edges, 0);
    assert_eq!(edges[0].source_package, "x");
    assert_eq!(edges[0].target_package, "y");
}

#[test]
fn workspace_graph_empty_for_single_project() {
    let db = Database::open_in_memory().unwrap();
    let edges = workspace_graph(&db).unwrap();
    assert!(edges.is_empty());
}

#[test]
fn workspace_graph_matches_dep_name_by_folder_fallback() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('core', 'crates/core', 'cargo')",
        [],
    )
    .unwrap();
    let core = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('cli', 'crates/cli', 'cargo')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind) \
         VALUES (?1, 'rust', 'cli', NULL, 'runtime')",
        rusqlite::params![core],
    )
    .unwrap();

    let edges = workspace_graph(&db).unwrap();
    assert_eq!(edges.len(), 1);
    assert!(edges[0].declared_dep);
    assert_eq!(edges[0].source_package, "core");
    assert_eq!(edges[0].target_package, "cli");
}

/// Two packages with the same display name in different paths must produce
/// two distinct graph rows. Earlier behavior keyed the aggregation on
/// `name` so they collapsed into one node.
#[test]
fn workspace_graph_keeps_two_same_named_packages_distinct() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    // Two packages both called "core" — one TS, one Rust.
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('core', 'apps/core', 'npm', '@org/core')",
        [],
    )
    .unwrap();
    let core_npm = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('core', 'crates/core', 'cargo', 'core')",
        [],
    )
    .unwrap();
    let core_cargo = conn.last_insert_rowid();

    // A caller package for each side.
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('web', 'apps/web', 'npm')",
        [],
    )
    .unwrap();
    let web = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('cli', 'crates/cli', 'cargo')",
        [],
    )
    .unwrap();
    let cli = conn.last_insert_rowid();

    // Files + symbols.
    let mk = |path: &str, lang: &str, hash: &str, pkg: i64| -> i64 {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) \
             VALUES (?1, ?2, ?3, 0, ?4)",
            rusqlite::params![path, hash, lang, pkg],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let mk_sym = |file: i64, name: &str| -> i64 {
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
             VALUES (?1, ?2, ?2, 'function', 1, 0)",
            rusqlite::params![file, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let web_f = mk("apps/web/src/i.ts", "typescript", "ha", web);
    let cli_f = mk("crates/cli/src/main.rs", "rust", "hb", cli);
    let core_ts_f = mk("apps/core/src/i.ts", "typescript", "hc", core_npm);
    let core_rs_f = mk("crates/core/src/lib.rs", "rust", "hd", core_cargo);

    let web_sym = mk_sym(web_f, "callTs");
    let cli_sym = mk_sym(cli_f, "callRs");
    let core_ts_sym = mk_sym(core_ts_f, "tsFn");
    let core_rs_sym = mk_sym(core_rs_f, "rsFn");

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) \
         VALUES (?1, ?2, 'calls', 1.0)",
        rusqlite::params![web_sym, core_ts_sym],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) \
         VALUES (?1, ?2, 'calls', 1.0)",
        rusqlite::params![cli_sym, core_rs_sym],
    )
    .unwrap();

    let edges = workspace_graph(&db).unwrap();
    assert_eq!(edges.len(), 2, "two `core` targets must produce two rows");

    let mut by_kind: std::collections::HashMap<Option<String>, &WorkspaceGraphEdge> =
        std::collections::HashMap::new();
    for e in &edges {
        by_kind.insert(e.target_package_kind.clone(), e);
    }
    let ts_row = by_kind.get(&Some("npm".to_string())).expect("npm row present");
    let rs_row = by_kind.get(&Some("cargo".to_string())).expect("cargo row present");
    assert_eq!(ts_row.source_package, "web");
    assert_eq!(ts_row.target_package_path, "apps/core");
    assert_eq!(rs_row.source_package, "cli");
    assert_eq!(rs_row.target_package_path, "crates/core");
}

/// A TypeScript package listing `shared` as a dependency must NOT bind to a
/// Cargo crate named `shared` in the same workspace — the ecosystem-aware
/// join in `workspace_graph` filters cross-language name collisions.
#[test]
fn workspace_graph_ecosystem_filter_rejects_cross_language_name_match() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('web', 'apps/web', 'npm', '@org/web')",
        [],
    )
    .unwrap();
    let web = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('shared', 'crates/shared', 'cargo', 'shared')",
        [],
    )
    .unwrap();

    // web (npm) declares a dep on `shared`. Only target is the cargo shared
    // crate — and the ecosystem filter should reject it.
    conn.execute(
        "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind) \
         VALUES (?1, 'typescript', 'shared', NULL, 'runtime')",
        rusqlite::params![web],
    )
    .unwrap();

    let edges = workspace_graph(&db).unwrap();
    assert!(
        edges.is_empty(),
        "cross-ecosystem name match must not produce a graph edge, got {edges:?}",
    );
}
