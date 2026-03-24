// =============================================================================
// search/hybrid.rs  —  FTS5 + vector search with Reciprocal Rank Fusion
//
// Three public entry points:
//
//   `hybrid_search`      — combines FTS5 trigram + KNN with RRF merging at
//                          chunk granularity (not file level)
//   `semantic_search`    — pure KNN, no text component
//   `rerank_references`  — re-orders `ReferenceResult` by semantic similarity
//                          to a definition context string
//
// Reciprocal Rank Fusion (k = 60):
//   rrf_score(d) = Σ  1 / (k + rank_i(d))
//   where rank_i is the 1-based position of document d in result list i.
//
// FTS5 returns file-level scores.  Every chunk belonging to a file that
// matched FTS5 inherits the file's FTS rank for RRF merging purposes.
// KNN returns chunk-level ranks directly.
//
// Vector search is silently skipped when sqlite-vec is not loaded; results
// fall back to FTS5-only with vector_rank absent from the RRF sum.
// =============================================================================

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace, warn};

use crate::db::Database;
use crate::search::content_search::search_content;
use crate::search::embedder::Embedder;
use crate::search::scope::SearchScope;
use crate::search::vector_store::knn_search;
use crate::types::ReferenceResult;

// ---------------------------------------------------------------------------
// RRF constant
// ---------------------------------------------------------------------------

const RRF_K: f64 = 60.0;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single result from hybrid or semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchResult {
    /// Relative file path (forward-slash).
    pub file_path: String,
    /// Name of the enclosing symbol, if the chunk is symbol-aligned.
    pub symbol_name: Option<String>,
    /// Symbol kind string (e.g. "function", "class"), if available.
    pub kind: Option<String>,
    /// 0-based start line of the chunk.
    pub start_line: u32,
    /// 0-based end line of the chunk.
    pub end_line: u32,
    /// Up to 200 chars of chunk content for display.
    pub content_preview: String,
    /// RRF combined score (higher is better).
    pub rrf_score: f64,
    /// 1-based rank in the FTS5 text results (None if not in text results).
    pub text_rank: Option<u32>,
    /// 1-based rank in the KNN vector results (None if not in vector results).
    pub vector_rank: Option<u32>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Hybrid search: combines FTS5 trigram text results with KNN vector results
/// using Reciprocal Rank Fusion at chunk granularity.
///
/// When sqlite-vec is not loaded, degrades gracefully to FTS5-only with
/// vector ranks omitted from the RRF sum.
pub fn hybrid_search(
    db: &Database,
    embedder: &mut Embedder,
    query: &str,
    scope: &SearchScope,
    limit: usize,
) -> Result<Vec<HybridSearchResult>> {
    let fetch_n = (limit * 2).max(20);

    // --- FTS5 text pass ---
    let text_results = search_content(db, query, scope, fetch_n)?;
    trace!(text_count = text_results.len(), query, "FTS5 text pass");

    // Map file_path → FTS rank (1-based).
    // All chunks belonging to a FTS-matching file inherit the file's rank.
    let text_file_rank: HashMap<String, u32> = text_results
        .iter()
        .enumerate()
        .map(|(i, r)| (r.file_path.clone(), (i + 1) as u32))
        .collect();

    // --- Vector pass (if sqlite-vec is loaded) ---
    let mut vec_chunk_rank: HashMap<i64, u32> = HashMap::new();

    if db.has_vec_extension() {
        match embedder.embed_query(query) {
            Ok(query_vec) => match knn_search(&db.conn, &query_vec, fetch_n) {
                Ok(knn_results) => {
                    trace!(vec_count = knn_results.len(), "KNN vector pass");
                    for (rank_0, (chunk_id, _dist)) in knn_results.iter().enumerate() {
                        vec_chunk_rank.insert(*chunk_id, (rank_0 + 1) as u32);
                    }
                }
                Err(e) => warn!("KNN search failed, falling back to text-only: {e}"),
            },
            Err(e) => warn!("embed_query failed, falling back to text-only: {e}"),
        }
    }

    // --- Collect candidate chunk IDs ---
    // Union of chunks from text-matching files and KNN-matched chunk IDs.
    let mut candidate_chunk_ids: Vec<i64> = Vec::new();

    for path in text_file_rank.keys() {
        let ids = chunk_ids_for_file_path(&db.conn, path)?;
        candidate_chunk_ids.extend(ids);
    }

    for &chunk_id in vec_chunk_rank.keys() {
        if !candidate_chunk_ids.contains(&chunk_id) {
            candidate_chunk_ids.push(chunk_id);
        }
    }

    candidate_chunk_ids.sort_unstable();
    candidate_chunk_ids.dedup();

    // --- RRF merge at chunk level ---
    let mut scored: Vec<(i64, f64, Option<u32>, Option<u32>)> = candidate_chunk_ids
        .iter()
        .filter_map(|&chunk_id| {
            // Determine this chunk's file path so we can look up its text rank.
            let file_path = chunk_file_path(&db.conn, chunk_id).ok()?;

            let text_rank = text_file_rank.get(&file_path).copied();
            let vec_rank = vec_chunk_rank.get(&chunk_id).copied();

            if text_rank.is_none() && vec_rank.is_none() {
                return None;
            }

            let rrf = rrf_score(text_rank, vec_rank);
            Some((chunk_id, rrf, text_rank, vec_rank))
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(fetch_n);

    // --- Build results with metadata ---
    let mut results: Vec<HybridSearchResult> = Vec::new();

    for (chunk_id, rrf, text_rank, vec_rank) in scored {
        let meta = match fetch_chunk_meta(&db.conn, chunk_id) {
            Ok(m) => m,
            Err(e) => {
                warn!(chunk_id, "Failed to fetch chunk metadata: {e}");
                continue;
            }
        };

        if !scope.matches_file(&meta.file_path, &meta.language) {
            continue;
        }

        results.push(HybridSearchResult {
            file_path: meta.file_path,
            symbol_name: meta.symbol_name,
            kind: meta.symbol_kind,
            start_line: meta.start_line,
            end_line: meta.end_line,
            content_preview: meta.content_preview,
            rrf_score: rrf,
            text_rank,
            vector_rank: vec_rank,
        });

        if results.len() >= limit {
            break;
        }
    }

    debug!(result_count = results.len(), query, "hybrid_search complete");
    Ok(results)
}

/// Pure semantic / vector search — no text component.
///
/// Returns an empty vec when sqlite-vec is not loaded.
pub fn semantic_search(
    db: &Database,
    embedder: &mut Embedder,
    query: &str,
    limit: usize,
) -> Result<Vec<HybridSearchResult>> {
    if !db.has_vec_extension() {
        debug!("semantic_search: sqlite-vec not loaded, returning empty");
        return Ok(vec![]);
    }

    let query_vec = embedder
        .embed_query(query)
        .context("embed_query failed in semantic_search")?;

    let knn = knn_search(&db.conn, &query_vec, limit)?;

    let mut results: Vec<HybridSearchResult> = Vec::new();

    for (rank_0, (chunk_id, _dist)) in knn.iter().enumerate() {
        let meta = match fetch_chunk_meta(&db.conn, *chunk_id) {
            Ok(m) => m,
            Err(e) => {
                warn!(chunk_id, "Failed to fetch chunk metadata: {e}");
                continue;
            }
        };

        let vector_rank = (rank_0 + 1) as u32;
        let rrf = 1.0 / (RRF_K + vector_rank as f64);

        results.push(HybridSearchResult {
            file_path: meta.file_path,
            symbol_name: meta.symbol_name,
            kind: meta.symbol_kind,
            start_line: meta.start_line,
            end_line: meta.end_line,
            content_preview: meta.content_preview,
            rrf_score: rrf,
            text_rank: None,
            vector_rank: Some(vector_rank),
        });

        trace!(chunk_id, rank = vector_rank, "semantic_search result");
    }

    debug!(result_count = results.len(), query, "semantic_search complete");
    Ok(results)
}

/// Re-rank `reference_results` by semantic similarity to `definition_context`.
///
/// Steps:
///   1. Embed `definition_context`.
///   2. For each reference, find the nearest code chunk covering `ref.line`
///      from the `code_chunks` table, embed its content.
///   3. Sort references by cosine similarity to the definition context,
///      descending.
///   4. Return the top `limit` references.
///
/// Falls back to returning the original order when the embedder fails.
pub fn rerank_references(
    db: &Database,
    embedder: &mut Embedder,
    reference_results: &[ReferenceResult],
    definition_context: &str,
    limit: usize,
) -> Result<Vec<ReferenceResult>> {
    if reference_results.is_empty() {
        return Ok(vec![]);
    }

    // Without an embedder model we cannot re-rank; return top `limit` as-is.
    if !embedder.is_loaded() {
        if let Err(e) = embedder.ensure_loaded() {
            warn!("rerank_references: embedder unavailable ({e}), returning original order");
            let mut out = reference_results.to_vec();
            out.truncate(limit);
            return Ok(out);
        }
    }

    let def_embedding = embedder
        .embed_query(definition_context)
        .context("Failed to embed definition context")?;

    // Collect context snippets for each reference.
    let ref_texts: Vec<String> = reference_results
        .iter()
        .map(|r| {
            chunk_snippet_for_line(&db.conn, &r.file_path, r.line)
                .unwrap_or_else(|| r.referencing_symbol.clone())
        })
        .collect();

    // Batch embed all reference contexts.
    let ref_text_refs: Vec<&str> = ref_texts.iter().map(|s| s.as_str()).collect();
    let ref_embeddings = match embedder.embed_documents(&ref_text_refs) {
        Ok(vecs) => vecs,
        Err(e) => {
            warn!("rerank_references: batch embed failed ({e}), returning original order");
            let mut out = reference_results.to_vec();
            out.truncate(limit);
            return Ok(out);
        }
    };

    let mut scored: Vec<(f32, usize)> = ref_embeddings
        .iter()
        .enumerate()
        .map(|(i, v)| (cosine_similarity(&def_embedding, v), i))
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let out: Vec<ReferenceResult> = scored
        .into_iter()
        .take(limit)
        .map(|(_, i)| reference_results[i].clone())
        .collect();

    debug!(result_count = out.len(), "rerank_references complete");
    Ok(out)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Compute the RRF score for a document given its optional ranks in each
/// result list.
fn rrf_score(text_rank: Option<u32>, vec_rank: Option<u32>) -> f64 {
    let mut score = 0.0f64;
    if let Some(r) = text_rank {
        score += 1.0 / (RRF_K + r as f64);
    }
    if let Some(r) = vec_rank {
        score += 1.0 / (RRF_K + r as f64);
    }
    score
}

/// Cosine similarity between two equal-length f32 slices.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Return all chunk IDs for the file at `file_path`.
fn chunk_ids_for_file_path(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT cc.id
         FROM code_chunks cc
         JOIN files f ON f.id = cc.file_id
         WHERE f.path = ?1",
    )?;
    let ids = stmt
        .query_map(params![file_path], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to query chunk ids for file")?;
    Ok(ids)
}

/// Return the file path for a given `chunk_id`.
fn chunk_file_path(conn: &rusqlite::Connection, chunk_id: i64) -> Result<String> {
    conn.query_row(
        "SELECT f.path
         FROM code_chunks cc
         JOIN files f ON f.id = cc.file_id
         WHERE cc.id = ?1",
        params![chunk_id],
        |row| row.get::<_, String>(0),
    )
    .context("Failed to look up file path for chunk")
}

/// Intermediate metadata row fetched for a chunk.
struct ChunkMeta {
    file_path: String,
    symbol_name: Option<String>,
    symbol_kind: Option<String>,
    start_line: u32,
    end_line: u32,
    content_preview: String,
    language: String,
}

/// Fetch display metadata for a chunk from `code_chunks` + `files` + `symbols`.
fn fetch_chunk_meta(conn: &rusqlite::Connection, chunk_id: i64) -> Result<ChunkMeta> {
    conn.query_row(
        "SELECT
             f.path,
             cc.content,
             cc.start_line,
             cc.end_line,
             f.language,
             s.name  AS symbol_name,
             s.kind  AS symbol_kind
         FROM code_chunks cc
         JOIN files f ON f.id = cc.file_id
         LEFT JOIN symbols s ON s.id = cc.symbol_id
         WHERE cc.id = ?1",
        params![chunk_id],
        |row| {
            let content: String = row.get(1)?;
            let preview: String = content.chars().take(200).collect();
            Ok(ChunkMeta {
                file_path: row.get(0)?,
                content_preview: preview,
                start_line: row.get(2)?,
                end_line: row.get(3)?,
                language: row.get(4)?,
                symbol_name: row.get(5)?,
                symbol_kind: row.get(6)?,
            })
        },
    )
    .context("Failed to fetch chunk metadata")
}

/// Return the content of the `code_chunks` row whose line range covers `line`
/// for the given file path.  Used as the context snippet for a reference site.
fn chunk_snippet_for_line(
    conn: &rusqlite::Connection,
    file_path: &str,
    line: u32,
) -> Option<String> {
    conn.query_row(
        "SELECT cc.content
         FROM code_chunks cc
         JOIN files f ON f.id = cc.file_id
         WHERE f.path = ?1
           AND cc.start_line <= ?2
           AND cc.end_line   >= ?2
         ORDER BY (cc.end_line - cc.start_line) ASC
         LIMIT 1",
        params![file_path, line],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        db.conn
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', ?2, 0)",
                params![path, language],
            )
            .unwrap();
        let file_id = db.conn.last_insert_rowid();

        // FTS5 content table row.
        db.conn
            .execute(
                "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
                params![file_id, path, content],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO code_chunks (file_id, content_hash, content, start_line, end_line)
                 VALUES (?1, 'h', ?2, 0, 5)",
                params![file_id, content],
            )
            .unwrap();
        let chunk_id = db.conn.last_insert_rowid();

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

        let db = Database::open_with_vec(std::path::Path::new(&db_path)).unwrap();
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
}
