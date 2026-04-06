use super::*;
use crate::db::Database;

fn make_db() -> Database {
    Database::open_in_memory().unwrap()
}

fn insert_file_with_chunk(
    db: &Database,
    path: &str,
    language: &str,
    content: &str,
) -> (i64, i64) {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', ?2, 0)",
            params![path, language],
        )
        .unwrap();
    let file_id = db.conn().last_insert_rowid();

    // FTS5 content table row.
    db.conn()
        .execute(
            "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
            params![file_id, path, content],
        )
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO code_chunks (file_id, content_hash, content, start_line, end_line)
             VALUES (?1, 'h', ?2, 0, 5)",
            params![file_id, content],
        )
        .unwrap();
    let chunk_id = db.conn().last_insert_rowid();

    (file_id, chunk_id)
}

// -----------------------------------------------------------------------
// cosine_similarity — pure function
// -----------------------------------------------------------------------

#[test]
fn cosine_similarity_identical_unit_vectors() {
    let v = vec![1.0f32, 0.0, 0.0];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
}

#[test]
fn cosine_similarity_orthogonal_vectors() {
    let a = vec![1.0f32, 0.0];
    let b = vec![0.0f32, 1.0];
    assert!(cosine_similarity(&a, &b).abs() < 1e-6);
}

#[test]
fn cosine_similarity_opposite_vectors() {
    let a = vec![1.0f32, 2.0, 3.0];
    let b = vec![-1.0f32, -2.0, -3.0];
    assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-5);
}

#[test]
fn cosine_similarity_zero_vector_returns_zero() {
    let a = vec![0.0f32; 4];
    let b = vec![1.0f32, 0.0, 0.0, 0.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn cosine_similarity_general_case() {
    let v = vec![1.0, 2.0, 3.0];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
}

// -----------------------------------------------------------------------
// rrf_score — pure function
// -----------------------------------------------------------------------

#[test]
fn rrf_score_both_ranks() {
    let score = rrf_score(Some(1), Some(1));
    let expected = 2.0 / (RRF_K + 1.0);
    assert!((score - expected).abs() < 1e-10);
}

#[test]
fn rrf_score_text_only() {
    let score = rrf_score(Some(5), None);
    let expected = 1.0 / (RRF_K + 5.0);
    assert!((score - expected).abs() < 1e-10);
}

#[test]
fn rrf_score_vector_only() {
    let score = rrf_score(None, Some(3));
    let expected = 1.0 / (RRF_K + 3.0);
    assert!((score - expected).abs() < 1e-10);
}

#[test]
fn rrf_score_neither_is_zero() {
    assert_eq!(rrf_score(None, None), 0.0);
}

#[test]
fn rrf_score_better_ranks_win() {
    let high = rrf_score(Some(1), Some(2));
    let low = rrf_score(Some(10), Some(20));
    assert!(high > low);
}

#[test]
fn rrf_score_two_lists_beats_one() {
    let both = rrf_score(Some(1), Some(1));
    let one = rrf_score(Some(1), None);
    assert!(both > one);
}

// -----------------------------------------------------------------------
// hybrid_search — structural tests without model / sqlite-vec
// -----------------------------------------------------------------------

#[test]
fn hybrid_search_degrades_to_fts_without_vec() {
    let db = make_db();
    insert_file_with_chunk(&db, "src/auth.rs", "rust", "fn authenticate_user() {}");

    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));
    let results =
        hybrid_search(&db, &mut embedder, "authenticate", &SearchScope::default(), 10)
            .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, "src/auth.rs");
    assert!(results[0].vector_rank.is_none());
    assert!(results[0].text_rank.is_some());
}

#[test]
fn hybrid_search_short_query_returns_empty() {
    let db = make_db();
    insert_file_with_chunk(&db, "x.rs", "rust", "fn ab() {}");

    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));
    let results =
        hybrid_search(&db, &mut embedder, "ab", &SearchScope::default(), 10).unwrap();

    // "ab" is < 3 chars — FTS5 trigram returns nothing.
    assert!(results.is_empty());
}

#[test]
fn hybrid_search_scope_filters_results() {
    let db = make_db();
    insert_file_with_chunk(&db, "src/a.rs", "rust", "fn needle_function() {}");
    insert_file_with_chunk(&db, "tests/b.rs", "rust", "fn needle_function() {}");

    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));
    let scope = SearchScope::default().with_directory("src");

    let results =
        hybrid_search(&db, &mut embedder, "needle_function", &scope, 10).unwrap();

    assert_eq!(results.len(), 1, "Scope should exclude tests/ file");
    assert!(results[0].file_path.starts_with("src/"));
}

#[test]
fn hybrid_search_rrf_score_is_positive() {
    let db = make_db();
    insert_file_with_chunk(&db, "src/x.rs", "rust", "fn target_search() {}");

    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));
    let results =
        hybrid_search(&db, &mut embedder, "target_search", &SearchScope::default(), 10)
            .unwrap();

    assert!(!results.is_empty());
    assert!(results[0].rrf_score > 0.0);
}

// -----------------------------------------------------------------------
// semantic_search — structural tests
// -----------------------------------------------------------------------

#[test]
fn semantic_search_returns_empty_without_vec() {
    let db = make_db();
    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));
    let results = semantic_search(&db, &mut embedder, "some query", 10).unwrap();
    assert!(results.is_empty());
}

// -----------------------------------------------------------------------
// rerank_references — structural tests
// -----------------------------------------------------------------------

#[test]
fn rerank_references_empty_input() {
    let db = make_db();
    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));
    let result = rerank_references(&db, &mut embedder, &[], "ctx", 10).unwrap();
    assert!(result.is_empty());
}

#[test]
fn rerank_references_falls_back_when_embedder_unavailable() {
    let db = make_db();
    let mut embedder = Embedder::new(std::path::PathBuf::from("/nonexistent"));

    let refs = vec![
        make_ref("FooService", "src/foo.rs", 10),
        make_ref("BarService", "src/bar.rs", 20),
        make_ref("BazService", "src/baz.rs", 30),
    ];

    let result =
        rerank_references(&db, &mut embedder, &refs, "definition context", 2).unwrap();

    // Embedder cannot load → original order, truncated to limit=2.
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].referencing_symbol, "FooService");
    assert_eq!(result[1].referencing_symbol, "BarService");
}

fn make_ref(symbol: &str, path: &str, line: u32) -> ReferenceResult {
    ReferenceResult {
        referencing_symbol: symbol.to_string(),
        referencing_kind: "function".to_string(),
        file_path: path.to_string(),
        line,
        edge_kind: "calls".to_string(),
        confidence: 1.0,
    }
}

// -----------------------------------------------------------------------
// Full integration tests — require model + sqlite-vec
// -----------------------------------------------------------------------

#[test]
#[ignore]
fn hybrid_search_with_full_stack() {
    let model_dir = std::env::var("ALPHAT_MODEL_DIR")
        .expect("Set ALPHAT_MODEL_DIR to run this test");
    let db_path = std::env::var("ALPHAT_TEST_DB")
        .expect("Set ALPHAT_TEST_DB to an indexed project DB");

    let db = Database::open(std::path::Path::new(&db_path)).unwrap();
    let mut embedder = Embedder::new(std::path::PathBuf::from(model_dir));

    let results = hybrid_search(
        &db,
        &mut embedder,
        "authenticate user login",
        &SearchScope::default(),
        10,
    )
    .unwrap();

    assert!(!results.is_empty());
    for r in &results {
        assert!(!r.file_path.is_empty());
        assert!(r.rrf_score > 0.0);
    }
}

#[test]
#[ignore]
fn rerank_references_with_model() {
    let model_dir = std::env::var("ALPHAT_MODEL_DIR")
        .expect("Set ALPHAT_MODEL_DIR to run this test");
    let db = make_db();
    let mut embedder = Embedder::new(std::path::PathBuf::from(model_dir));

    insert_file_with_chunk(&db, "src/auth.rs", "rust", "fn authenticate(user: &User) -> bool { user.is_active() }");
    insert_file_with_chunk(&db, "src/unrelated.rs", "rust", "fn format_date(ts: i64) -> String { ts.to_string() }");

    let refs = vec![
        make_ref("authenticate", "src/auth.rs", 0),
        make_ref("format_date", "src/unrelated.rs", 0),
    ];

    let reranked = rerank_references(
        &db,
        &mut embedder,
        &refs,
        "fn authenticate(user: &User) -> bool",
        2,
    )
    .unwrap();

    assert_eq!(reranked.len(), 2);
    // The authentication function should rank first.
    assert_eq!(reranked[0].referencing_symbol, "authenticate");
}
