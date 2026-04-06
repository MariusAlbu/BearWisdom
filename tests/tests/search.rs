//! Integration tests for the search engine (grep, content search, fuzzy).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bearwisdom::search::content_index::rebuild_content_index;
use bearwisdom::search::content_search::search_content;
use bearwisdom::search::fuzzy::FuzzyIndex;
use bearwisdom::search::grep::{grep_search, GrepOptions};
use bearwisdom::search::scope::SearchScope;
use bearwisdom::full_index;
use bearwisdom_tests::TestProject;

fn cancel_never() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

// ── grep ────────────────────────────────────────────────────────────────

#[test]
fn grep_finds_literal_match() {
    let project = TestProject::csharp_service();
    let cancel = cancel_never();

    let results = grep_search(
        project.path(),
        "IProductRepository",
        &GrepOptions::default(),
        &cancel,
    )
    .unwrap();

    assert!(!results.is_empty(), "should find IProductRepository in C# files");

    // At least one match should be in the interface definition file.
    let in_interface = results
        .iter()
        .any(|m| m.file_path.contains("IProductRepository"));
    assert!(in_interface, "should match in the interface file");
}

#[test]
fn grep_case_insensitive() {
    let project = TestProject::python_app();
    let cancel = cancel_never();

    let options = GrepOptions {
        case_sensitive: false,
        ..Default::default()
    };

    let results = grep_search(project.path(), "animal", &options, &cancel).unwrap();
    assert!(!results.is_empty(), "case-insensitive search for 'animal' should match 'Animal'");
}

#[test]
fn grep_respects_scope_language_filter() {
    let project = TestProject::multi_lang();
    let cancel = cancel_never();

    let options = GrepOptions {
        scope: SearchScope::new().with_language("python"),
        ..Default::default()
    };

    let results = grep_search(project.path(), "def ", &options, &cancel).unwrap();

    for m in &results {
        assert!(
            m.file_path.ends_with(".py"),
            "language-scoped grep should only return .py files, got: {}",
            m.file_path,
        );
    }
}

#[test]
fn grep_no_results_for_absent_string() {
    let project = TestProject::csharp_service();
    let cancel = cancel_never();

    let results = grep_search(
        project.path(),
        "ThisStringDoesNotExistAnywhere",
        &GrepOptions::default(),
        &cancel,
    )
    .unwrap();

    assert!(results.is_empty());
}

#[test]
fn grep_respects_cancellation() {
    let project = TestProject::csharp_service();
    let cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled

    let results = grep_search(
        project.path(),
        "class",
        &GrepOptions::default(),
        &cancel,
    )
    .unwrap();

    // With cancellation set before start, should return 0 or very few results.
    // The exact behavior depends on whether the first file is checked before cancel.
    assert!(results.len() <= 1, "cancelled search should return at most 1 result");
}

// ── content search (FTS5) ───────────────────────────────────────────────

#[test]
fn content_search_after_indexing() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    // Rebuild the FTS content index from the indexed files.
    let indexed = rebuild_content_index(db.conn(), project.path()).unwrap();
    assert!(indexed > 0, "should index content from at least one file");

    let results = search_content(&db, "ProductService", &SearchScope::default(), 10).unwrap();
    assert!(!results.is_empty(), "FTS should find 'ProductService'");
}

#[test]
fn content_search_short_query_returns_empty() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();
    rebuild_content_index(db.conn(), project.path()).unwrap();

    // Queries shorter than 3 chars return empty (trigram minimum).
    let results = search_content(&db, "ab", &SearchScope::default(), 10).unwrap();
    assert!(results.is_empty(), "sub-trigram queries should return empty");
}

// ── fuzzy search ────────────────────────────────────────────────────────

#[test]
fn fuzzy_match_files() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let index = FuzzyIndex::from_db(&db).unwrap();
    let matches = index.match_files("ProdServ", 10);

    assert!(!matches.is_empty(), "fuzzy file search for 'ProdServ' should match ProductService.cs");
}

#[test]
fn fuzzy_match_symbols() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let index = FuzzyIndex::from_db(&db).unwrap();
    let matches = index.match_symbols("GetById", 10);

    assert!(!matches.is_empty(), "fuzzy symbol search for 'GetById' should find the method");
}

#[test]
fn fuzzy_empty_query_returns_nothing() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let index = FuzzyIndex::from_db(&db).unwrap();
    assert!(index.match_files("", 10).is_empty());
    assert!(index.match_symbols("", 10).is_empty());
}
