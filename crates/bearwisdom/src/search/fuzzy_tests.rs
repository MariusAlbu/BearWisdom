use super::*;
use crate::db::Database;

fn make_db_with_data() -> Database {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    // Insert three files.
    for (path, lang) in [
        ("src/catalog/CatalogService.ts", "typescript"),
        ("src/orders/OrderRepository.ts", "typescript"),
        ("src/shared/utils.ts", "typescript"),
    ] {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![path, format!("hash_{path}"), lang],
        )
        .unwrap();
    }

    // Insert symbols for the first file.
    let file_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE path = 'src/catalog/CatalogService.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    for (name, qname, kind, line) in [
        ("fetchCatalog", "CatalogService.fetchCatalog", "function", 10u32),
        ("CatalogService", "CatalogService", "class", 1),
        ("updateItem", "CatalogService.updateItem", "method", 42),
    ] {
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            rusqlite::params![file_id, name, qname, kind, line],
        )
        .unwrap();
    }

    db
}

#[test]
fn loads_from_db() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();
    assert_eq!(idx.file_count(), 3);
    assert_eq!(idx.symbol_count(), 3);
}

#[test]
fn match_files_returns_sorted_results() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();

    let results = idx.match_files("catalog", 10);
    assert!(!results.is_empty(), "Expected at least one file match for 'catalog'");

    // The catalog file should appear and score highest.
    assert!(
        results[0].text.contains("catalog"),
        "Expected catalog file to rank first, got: {}",
        results[0].text
    );

    // Scores should be in descending order.
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Results not sorted by score: {} < {}",
            window[0].score,
            window[1].score
        );
    }
}

#[test]
fn match_files_respects_limit() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();

    // All three files contain 'src'; limit to 2.
    let results = idx.match_files("src", 2);
    assert!(results.len() <= 2);
}

#[test]
fn match_files_empty_pattern_returns_empty() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();
    assert!(idx.match_files("", 10).is_empty());
}

#[test]
fn match_symbols_finds_by_prefix() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();

    let results = idx.match_symbols("fetchCatalog", 10);
    assert!(!results.is_empty(), "Expected match for 'fetchCatalog'");
    assert!(results[0].text.contains("fetchCatalog"));
}

#[test]
fn match_symbols_metadata_populated() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();

    let results = idx.match_symbols("CatalogService", 10);
    assert!(!results.is_empty());

    let first = &results[0];
    match &first.metadata {
        FuzzyMetadata::Symbol { kind, file_path, line } => {
            assert_eq!(kind, "class");
            assert!(file_path.contains("CatalogService"));
            assert_eq!(*line, 1);
        }
        _ => panic!("Expected Symbol metadata"),
    }
}

#[test]
fn match_symbols_file_metadata_absent() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();

    // File matches should have File metadata.
    let results = idx.match_files("utils", 10);
    assert!(!results.is_empty());
    match &results[0].metadata {
        FuzzyMetadata::File { language } => {
            assert_eq!(language, "typescript");
        }
        _ => panic!("Expected File metadata"),
    }
}

#[test]
fn match_symbols_returns_indices_for_highlighting() {
    let db = make_db_with_data();
    let idx = FuzzyIndex::from_db(&db).unwrap();

    let results = idx.match_symbols("update", 5);
    assert!(!results.is_empty());
    // Indices should be non-empty (matched character positions).
    assert!(
        !results[0].indices.is_empty(),
        "Expected non-empty indices for highlight"
    );
}

#[test]
fn empty_index_returns_empty_results() {
    let db = Database::open_in_memory().unwrap();
    let idx = FuzzyIndex::from_db(&db).unwrap();
    assert_eq!(idx.file_count(), 0);
    assert!(idx.match_files("foo", 10).is_empty());
    assert!(idx.match_symbols("foo", 10).is_empty());
}
