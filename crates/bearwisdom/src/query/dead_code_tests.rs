use super::*;
use crate::db::Database;

fn setup_db() -> Database {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) \
         VALUES ('src/lib.rs', 'h1', 'rust', 0)",
        [],
    )
    .unwrap();
    let file_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'used_fn', 'mod::used_fn', 'function', 10, 0, 'public', 3)",
        [file_id],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'dead_fn', 'mod::dead_fn', 'function', 20, 0, 'private', 0)",
        [file_id],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'main', 'main', 'function', 1, 0, NULL, 0)",
        [file_id],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) \
         VALUES ('src/tests/test_lib.rs', 'h2', 'rust', 0)",
        [],
    )
    .unwrap();
    let test_file_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'test_something', 'tests::test_something', 'function', 5, 0, NULL, 0)",
        [test_file_id],
    )
    .unwrap();

    db
}

#[test]
fn dead_code_finds_uncalled_private_fn() {
    let db = setup_db();
    let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

    assert_eq!(report.dead_candidates.len(), 1);
    assert_eq!(report.dead_candidates[0].name, "dead_fn");
    assert_eq!(report.dead_candidates[0].confidence, 1.0);
}

#[test]
fn dead_code_excludes_main() {
    let db = setup_db();
    let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

    assert!(report.entry_points_excluded > 0);
    assert!(!report.dead_candidates.iter().any(|c| c.name == "main"));
}

#[test]
fn dead_code_excludes_test_files() {
    let db = setup_db();
    let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

    assert!(report.test_symbols_excluded > 0);
    assert!(!report
        .dead_candidates
        .iter()
        .any(|c| c.name == "test_something"));
}

#[test]
fn dead_code_includes_tests_when_asked() {
    let db = setup_db();
    let opts = DeadCodeOptions { include_tests: true, ..Default::default() };
    let report = find_dead_code(&db, &opts).unwrap();

    assert!(report
        .dead_candidates
        .iter()
        .any(|c| c.name == "test_something"));
}

#[test]
fn dead_code_respects_visibility_filter() {
    let db = setup_db();
    let opts = DeadCodeOptions {
        visibility_filter: VisibilityFilter::PublicOnly,
        ..Default::default()
    };
    let report = find_dead_code(&db, &opts).unwrap();
    assert!(!report.dead_candidates.iter().any(|c| c.name == "dead_fn"));
}

#[test]
fn entry_points_finds_main() {
    let db = setup_db();
    let report = find_entry_points(&db).unwrap();
    assert!(report
        .entry_points
        .iter()
        .any(|ep| ep.name == "main" && matches!(ep.entry_kind, EntryPointKind::Main)));
}

#[test]
fn entry_points_finds_test_functions() {
    let db = setup_db();
    let report = find_entry_points(&db).unwrap();
    assert!(report.entry_points.iter().any(|ep| ep.name == "test_something"
        && matches!(ep.entry_kind, EntryPointKind::TestFunction)));
}

#[test]
fn empty_db_returns_empty_report() {
    let db = Database::open_in_memory().unwrap();
    let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();
    assert!(report.dead_candidates.is_empty());
    assert_eq!(report.total_symbols_checked, 0);
}

/// A public symbol inside a package with a declared manifest name is library
/// API surface — `find_entry_points` must classify it as `ExportedApi`, and
/// `find_dead_code` must exclude it from candidates.
#[test]
fn exported_api_recognized_inside_declared_package() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('libfoo', 'crates/libfoo', 'cargo', 'libfoo')",
        [],
    )
    .unwrap();
    let pkg = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, package_id) \
         VALUES ('crates/libfoo/src/lib.rs', 'h1', 'rust', 0, ?1)",
        [pkg],
    )
    .unwrap();
    let file = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'public_api_fn', 'libfoo::public_api_fn', 'function', 1, 0, 'public', 0)",
        [file],
    )
    .unwrap();

    let entry = find_entry_points(&db).unwrap();
    assert!(
        entry
            .entry_points
            .iter()
            .any(|ep| ep.name == "public_api_fn"
                && matches!(ep.entry_kind, EntryPointKind::ExportedApi)),
        "expected an ExportedApi entry, got {:?}",
        entry.entry_points
    );

    let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();
    assert!(
        !report
            .dead_candidates
            .iter()
            .any(|c| c.name == "public_api_fn"),
        "public symbol in a library package must not be flagged dead",
    );
}

/// `--scope` matched against a package's `declared_name` must restrict
/// candidates to files in that package, not require a path prefix.
#[test]
fn dead_code_scope_matches_package_declared_name() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('a', 'pkg-a', 'cargo', '@org/a')",
        [],
    )
    .unwrap();
    let pkg_a = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO packages (name, path, kind, declared_name) \
         VALUES ('b', 'pkg-b', 'cargo', '@org/b')",
        [],
    )
    .unwrap();
    let pkg_b = conn.last_insert_rowid();

    let mk = |path: &str, hash: &str, pkg: i64| -> i64 {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) \
             VALUES (?1, ?2, 'rust', 0, ?3)",
            rusqlite::params![path, hash, pkg],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let fa = mk("pkg-a/src/lib.rs", "h1", pkg_a);
    let fb = mk("pkg-b/src/lib.rs", "h2", pkg_b);

    // One dead private function in each package.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'dead_in_a', 'a::dead_in_a', 'function', 1, 0, 'private', 0)",
        [fa],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count) \
         VALUES (?1, 'dead_in_b', 'b::dead_in_b', 'function', 1, 0, 'private', 0)",
        [fb],
    )
    .unwrap();

    let opts = DeadCodeOptions { scope: Some("@org/a".to_string()), ..Default::default() };
    let report = find_dead_code(&db, &opts).unwrap();

    let names: Vec<&str> = report
        .dead_candidates
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"dead_in_a"), "scope should include a's dead symbol, got {names:?}");
    assert!(!names.contains(&"dead_in_b"), "scope should exclude b's dead symbol, got {names:?}");
}
