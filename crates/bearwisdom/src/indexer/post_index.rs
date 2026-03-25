// =============================================================================
// indexer/post_index.rs  —  post-index embedding pipeline
//
// After full_index / incremental_index creates code_chunks, this module
// computes CodeRankEmbed vectors and upserts them into the sqlite-vec
// virtual table.  All functions degrade gracefully when the ONNX model or
// sqlite-vec extension is unavailable.
// =============================================================================

use crate::search::embedder::Embedder;
use crate::search::vector_store;
use anyhow::Result;
use rusqlite::Connection;
use tracing::info;

/// Embed all code chunks that don't yet have a vector in `vec_chunks`.
///
/// Queries `code_chunks LEFT JOIN vec_chunks` to find un-embedded chunks,
/// embeds them in batches via the ONNX model, and upserts the vectors.
///
/// Returns `(embedded_count, skipped_count)`.  Skipped means the chunk
/// already had a vector.
///
/// Graceful degradation:
/// - Returns `(0, 0)` if sqlite-vec is not loaded (`vec_chunks` missing).
/// - Returns an error only if the embedder fails mid-batch.
pub fn embed_chunks(
    conn: &Connection,
    embedder: &mut Embedder,
    batch_size: usize,
) -> Result<(u32, u32)> {
    if !vector_store::vec_table_exists(conn) {
        match vector_store::init_vec_table(conn) {
            Ok(_) => info!("Created vec_chunks table"),
            Err(e) => {
                info!("sqlite-vec not available ({e:#}), skipping embedding");
                return Ok((0, 0));
            }
        }
    }

    // Find chunks that have no vector yet.
    let mut stmt = conn.prepare(
        "SELECT c.id, c.content
         FROM code_chunks c
         LEFT JOIN vec_chunks v ON v.chunk_id = c.id
         WHERE v.chunk_id IS NULL",
    )?;

    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let total = rows.len() as u32;
    if total == 0 {
        info!("All chunks already embedded, nothing to do");
        return Ok((0, total));
    }

    // Load the model only when there are chunks to embed.
    embedder.ensure_loaded()?;

    info!("Embedding {total} un-embedded chunks (batch_size={batch_size})");

    let effective_batch = if batch_size == 0 { 32 } else { batch_size };
    let mut embedded = 0u32;

    for batch in rows.chunks(effective_batch) {
        let texts: Vec<&str> = batch.iter().map(|(_, content)| content.as_str()).collect();
        let vectors = embedder.embed_documents(&texts)?;

        let pairs: Vec<(i64, &[f32])> = batch
            .iter()
            .zip(vectors.iter())
            .map(|((id, _), vec)| (*id, vec.as_slice()))
            .collect();

        vector_store::upsert_vectors(conn, &pairs)?;
        embedded += pairs.len() as u32;
    }

    info!("Embedded {embedded} chunks ({} already had vectors)", total.saturating_sub(embedded));
    Ok((embedded, 0))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn embed_chunks_no_vec_extension() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let mut embedder = Embedder::new(PathBuf::from("/nonexistent"));
        // Should return (0, 0) gracefully — no sqlite-vec loaded.
        let result = embed_chunks(&db.conn, &mut embedder, 32).unwrap();
        assert_eq!(result, (0, 0));
    }
}
