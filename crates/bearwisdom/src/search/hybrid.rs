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

use std::collections::{HashMap, HashSet};

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
            Ok(query_vec) => match knn_search(db.conn(), &query_vec, fetch_n) {
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
    // Batch-fetch all chunk IDs for the FTS-matched files in one query rather
    // than one query per file path.
    let fts_paths: Vec<&str> = text_file_rank.keys().map(|s| s.as_str()).collect();
    let mut candidate_chunk_ids: HashSet<i64> =
        batch_chunk_ids_for_files(db.conn(), &fts_paths)?;

    for &chunk_id in vec_chunk_rank.keys() {
        candidate_chunk_ids.insert(chunk_id);
    }

    let mut candidate_chunk_ids: Vec<i64> = candidate_chunk_ids.into_iter().collect();
    candidate_chunk_ids.sort_unstable();

    // --- Batch-fetch metadata for all candidates (1 query) ---
    // file_path is available in the metadata, so we do not need a separate
    // chunk→file_path pass; this single batch covers both the RRF scoring step
    // and the final result-building step.
    let meta_map = batch_fetch_chunk_meta(db.conn(), &candidate_chunk_ids)?;

    // --- RRF merge at chunk level ---
    let mut scored: Vec<(i64, f64, Option<u32>, Option<u32>)> = candidate_chunk_ids
        .iter()
        .filter_map(|&chunk_id| {
            let meta = meta_map.get(&chunk_id)?;

            let text_rank = text_file_rank.get(&meta.file_path).copied();
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

    // --- Build results from the already-fetched metadata map ---
    let mut results: Vec<HybridSearchResult> = Vec::new();

    for (chunk_id, rrf, text_rank, vec_rank) in scored {
        let meta = match meta_map.get(&chunk_id) {
            Some(m) => m,
            None => {
                warn!(chunk_id, "chunk_id disappeared from meta_map during result build");
                continue;
            }
        };

        if !scope.matches_file(&meta.file_path, &meta.language) {
            continue;
        }

        results.push(HybridSearchResult {
            file_path: meta.file_path.clone(),
            symbol_name: meta.symbol_name.clone(),
            kind: meta.symbol_kind.clone(),
            start_line: meta.start_line,
            end_line: meta.end_line,
            content_preview: meta.content_preview.clone(),
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

    let query_vec = match embedder.embed_query(query) {
        Ok(v) => v,
        Err(e) => {
            debug!("semantic_search: embedder unavailable ({e:#}), returning empty");
            return Ok(vec![]);
        }
    };

    let knn = knn_search(db.conn(), &query_vec, limit)?;

    let knn_ids: Vec<i64> = knn.iter().map(|(id, _)| *id).collect();
    let meta_map = batch_fetch_chunk_meta(db.conn(), &knn_ids)?;

    let mut results: Vec<HybridSearchResult> = Vec::new();

    for (rank_0, (chunk_id, _dist)) in knn.iter().enumerate() {
        let meta = match meta_map.get(chunk_id) {
            Some(m) => m,
            None => {
                warn!(chunk_id, "Failed to fetch chunk metadata");
                continue;
            }
        };

        let vector_rank = (rank_0 + 1) as u32;
        let rrf = 1.0 / (RRF_K + vector_rank as f64);

        results.push(HybridSearchResult {
            file_path: meta.file_path.clone(),
            symbol_name: meta.symbol_name.clone(),
            kind: meta.symbol_kind.clone(),
            start_line: meta.start_line,
            end_line: meta.end_line,
            content_preview: meta.content_preview.clone(),
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
            chunk_snippet_for_line(db.conn(), &r.file_path, r.line)
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

/// Batch-fetch all chunk IDs that belong to any of the given file paths.
///
/// Uses a temp table to avoid a per-path round-trip.  Returns the union of
/// chunk IDs across all supplied paths.  An empty input returns an empty set.
fn batch_chunk_ids_for_files(
    conn: &rusqlite::Connection,
    file_paths: &[&str],
) -> Result<HashSet<i64>> {
    if file_paths.is_empty() {
        return Ok(HashSet::new());
    }

    conn.execute(
        "CREATE TEMP TABLE IF NOT EXISTS _search_paths (path TEXT PRIMARY KEY)",
        [],
    )?;
    conn.execute("DELETE FROM _search_paths", [])?;

    {
        let mut ins =
            conn.prepare("INSERT OR IGNORE INTO _search_paths (path) VALUES (?1)")?;
        for path in file_paths {
            ins.execute([*path])?;
        }
    }

    let mut stmt = conn.prepare(
        "SELECT cc.id
         FROM code_chunks cc
         JOIN files f ON f.id = cc.file_id
         JOIN _search_paths sp ON sp.path = f.path",
    )?;
    let ids: HashSet<i64> = stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .collect();

    conn.execute("DELETE FROM _search_paths", [])?;
    Ok(ids)
}

/// Batch-fetch display metadata for a set of chunk IDs in a single query.
///
/// Uses a temp table to avoid a per-chunk round-trip.  Returns a map from
/// chunk ID to `ChunkMeta`.  Missing chunk IDs are silently absent from the
/// result (callers should warn on a cache miss).
fn batch_fetch_chunk_meta(
    conn: &rusqlite::Connection,
    chunk_ids: &[i64],
) -> Result<HashMap<i64, ChunkMeta>> {
    if chunk_ids.is_empty() {
        return Ok(HashMap::new());
    }

    conn.execute(
        "CREATE TEMP TABLE IF NOT EXISTS _search_chunks (id INTEGER PRIMARY KEY)",
        [],
    )?;
    conn.execute("DELETE FROM _search_chunks", [])?;

    {
        let mut ins =
            conn.prepare("INSERT OR IGNORE INTO _search_chunks (id) VALUES (?1)")?;
        for &id in chunk_ids {
            ins.execute([id])?;
        }
    }

    let mut stmt = conn.prepare(
        "SELECT cc.id,
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
         JOIN _search_chunks sc ON sc.id = cc.id",
    )?;

    let map: HashMap<i64, ChunkMeta> = stmt
        .query_map([], |row| {
            let chunk_id: i64 = row.get(0)?;
            let content: String = row.get(2)?;
            let preview: String = content.chars().take(200).collect();
            Ok((
                chunk_id,
                ChunkMeta {
                    file_path: row.get(1)?,
                    content_preview: preview,
                    start_line: row.get(3)?,
                    end_line: row.get(4)?,
                    language: row.get(5)?,
                    symbol_name: row.get(6)?,
                    symbol_kind: row.get(7)?,
                },
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    conn.execute("DELETE FROM _search_chunks", [])?;
    Ok(map)
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
#[path = "hybrid_tests.rs"]
mod tests;
