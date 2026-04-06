use super::*;
use crate::db::Database;
use rusqlite::Connection;

fn insert_file_and_content(conn: &Connection, path: &str, language: &str, content: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', ?2, 0)",
        rusqlite::params![path, language],
    )
    .unwrap();
    let id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, path, content],
    )
    .unwrap();

    id
}

#[test]
fn search_finds_matching_file() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "src/service.rs", "rust", "fn authenticate_user() {}");
    insert_file_and_content(db.conn(), "src/other.rs", "rust", "fn unrelated() {}");

    let results =
        search_content(&db, "authenticate", &SearchScope::default(), 10).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, "src/service.rs");
}

#[test]
fn query_shorter_than_three_chars_returns_empty() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "a.rs", "rust", "ab content");

    let results = search_content(&db, "ab", &SearchScope::default(), 10).unwrap();
    assert!(results.is_empty(), "< 3 chars should return empty");
}

#[test]
fn search_returns_empty_when_no_match() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "x.rs", "rust", "fn hello() {}");

    let results = search_content(&db, "zzznomatch", &SearchScope::default(), 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn scope_language_filter_applied_after_fts() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "logic.rs", "rust", "fn process_order() {}");
    insert_file_and_content(
        db.conn(),
        "logic.ts",
        "typescript",
        "function processOrder() {}",
    );

    let scope = SearchScope::default().with_language("rust");
    let results = search_content(&db, "process", &scope, 10).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].language, "rust");
}

#[test]
fn score_is_positive() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "score.rs", "rust", "fn important_function() {}");

    let results =
        search_content(&db, "important", &SearchScope::default(), 10).unwrap();

    assert_eq!(results.len(), 1);
    assert!(
        results[0].score >= 0.0,
        "score should be non-negative, got {}",
        results[0].score
    );
}

#[test]
fn limit_respected() {
    let db = Database::open_in_memory().unwrap();
    for i in 0..10 {
        insert_file_and_content(
            db.conn(),
            &format!("file{i}.rs"),
            "rust",
            "fn needle() {}",
        );
    }

    let results = search_content(&db, "needle", &SearchScope::default(), 3).unwrap();
    assert!(results.len() <= 3);
}

#[test]
fn multiple_files_all_returned() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "a.rs", "rust", "fn shared_name() {}");
    insert_file_and_content(db.conn(), "b.rs", "rust", "fn shared_name_too() {}");
    insert_file_and_content(db.conn(), "c.rs", "rust", "nothing here");

    let results =
        search_content(&db, "shared_name", &SearchScope::default(), 10).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn scope_directory_filter_applied() {
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(db.conn(), "src/foo.rs", "rust", "fn needle() {}");
    insert_file_and_content(db.conn(), "tests/bar.rs", "rust", "fn needle() {}");

    let scope = SearchScope::default().with_directory("src");
    let results = search_content(&db, "needle", &scope, 10).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].file_path.starts_with("src/"));
}

#[test]
fn quote_fts_query_single_word() {
    let q = quote_fts_query("budget");
    assert_eq!(q, r#""budget""#);
}

#[test]
fn quote_fts_query_multi_word_uses_or() {
    // IDE-040: multi-word queries must be OR-joined so each token can match
    // independently rather than requiring the phrase to appear contiguously.
    let q = quote_fts_query("budget service");
    assert_eq!(q, r#""budget" OR "service""#);
}

#[test]
fn quote_fts_query_escapes_double_quotes() {
    // A token containing a literal double-quote has it doubled per FTS5 rules.
    // The token `"hello"` (with surrounding quotes) becomes `"""hello"""`.
    let q = quote_fts_query(r#""hello""#);
    assert_eq!(q, r#""""hello""""#);
}

#[test]
fn quote_fts_query_skips_short_tokens() {
    // Tokens < 3 chars are below the trigram minimum and produce no FTS match.
    let q = quote_fts_query("a bb catalog");
    assert_eq!(q, r#""catalog""#);
}

#[test]
fn quote_fts_query_all_short_returns_empty_literal() {
    let q = quote_fts_query("a bb");
    assert_eq!(q, r#""""#);
}

#[test]
fn hybrid_search_multi_word_query_returns_results() {
    // IDE-040 regression: "budget service" should match files containing
    // either "budget" or "service" individually (OR semantics).
    let db = Database::open_in_memory().unwrap();
    insert_file_and_content(
        db.conn(),
        "src/budget_service.rs",
        "rust",
        "pub struct BudgetService { balance: f64 }",
    );

    let results = search_content(&db, "budget service", &SearchScope::default(), 10).unwrap();
    assert!(
        !results.is_empty(),
        "multi-word query should return results via OR semantics"
    );
    assert_eq!(results[0].file_path, "src/budget_service.rs");
}

#[test]
fn search_content_with_lines_returns_grep_matches() {
    use std::io::Write;
    use tempfile::TempDir;

    let root = TempDir::new().unwrap();
    let db = Database::open_in_memory().unwrap();

    // Write a real file to disk.
    let rel = "src/catalog.rs";
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    let mut f = std::fs::File::create(root.path().join(rel)).unwrap();
    f.write_all(b"fn get_catalog_item() -> CatalogItem {\n    todo!()\n}\n")
        .unwrap();

    insert_file_and_content(
        db.conn(),
        rel,
        "rust",
        "fn get_catalog_item() -> CatalogItem {\n    todo!()\n}\n",
    );

    let matches = search_content_with_lines(
        &db,
        root.path(),
        "CatalogItem",
        &SearchScope::default(),
        50,
    )
    .unwrap();

    assert!(!matches.is_empty(), "Should find at least one line match");
    assert!(
        matches.iter().any(|m| m.line_content.contains("CatalogItem")),
        "Match content should contain the search term"
    );
}
