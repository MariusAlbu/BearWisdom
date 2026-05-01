use super::pattern::{pattern_search, PatternMatch};
use crate::db::Database;
use std::path::Path;

fn insert_file(db: &Database, path: &str, language: &str) {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, origin)
         VALUES (?1, 'h', ?2, 0, 'internal')
         ON CONFLICT(path) DO NOTHING",
        rusqlite::params![path, language],
    )
    .unwrap();
}

#[test]
fn pattern_search_finds_function_definitions_in_rust_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let rel = "src/lib.rs";
    let abs = root.join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(
        &abs,
        "pub fn alpha() {}\npub fn beta() {}\npub struct Gamma;\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    insert_file(&db, rel, "rust");

    let query = "(function_item name: (identifier) @fn)";
    let results: Vec<PatternMatch> =
        pattern_search(&db, root, "rust", query, 10).expect("pattern_search ok");

    let names: Vec<&str> = results.iter().map(|m| m.snippet.as_str()).collect();
    assert!(names.contains(&"alpha"), "expected `alpha` capture, got {names:?}");
    assert!(names.contains(&"beta"), "expected `beta` capture, got {names:?}");
    assert_eq!(results.len(), 2, "expected exactly 2 fn captures, got {}", results.len());
    for m in &results {
        assert_eq!(m.capture_name, "fn");
        assert_eq!(m.file_path, rel);
    }
}

#[test]
fn pattern_search_respects_max_results() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let rel = "src/lib.rs";
    let abs = root.join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(
        &abs,
        "pub fn a() {}\npub fn b() {}\npub fn c() {}\npub fn d() {}\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    insert_file(&db, rel, "rust");

    let results = pattern_search(
        &db,
        root,
        "rust",
        "(function_item name: (identifier) @fn)",
        2,
    )
    .unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn pattern_search_invalid_query_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    let result = pattern_search(&db, dir.path(), "rust", "(invalid_node_kind)", 10);
    assert!(result.is_err(), "expected error for invalid query");
}

#[test]
fn pattern_search_unknown_language_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_in_memory().unwrap();
    // Use a language string that has no registered grammar; the registry
    // returns the fallback plugin which has `grammar()` returning None.
    let result = pattern_search(&db, dir.path(), "no-such-lang", "(_) @x", 10);
    assert!(result.is_err());
}

// Suppress unused-import warning when this file builds in isolation.
#[allow(dead_code)]
fn _unused(_: &Path) {}
