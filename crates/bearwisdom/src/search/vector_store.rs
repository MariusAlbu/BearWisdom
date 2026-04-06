// =============================================================================
// search/vector_store.rs  —  sqlite-vec vector storage and KNN search
//
// Manages the `vec_chunks` virtual table (created by the sqlite-vec extension)
// for storing 768-dimensional code embeddings and performing k-nearest-neighbor
// cosine similarity searches.
//
// The sqlite-vec extension is statically linked and loaded automatically by
// `Database::open()`.  If the extension is unavailable, all functions return
// graceful errors or empty results.
//
// Wire format: little-endian IEEE 754 f32 bytes — the native format expected
// by sqlite-vec for both INSERT and WHERE MATCH.
// =============================================================================

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::{debug, trace};

// ---------------------------------------------------------------------------
// Table management
// ---------------------------------------------------------------------------

/// Create the `vec_chunks` virtual table.
///
/// Requires sqlite-vec to be loaded (available on any connection from `Database::open`).
/// Returns `Ok(true)` if the table was created, `Ok(false)` if it already
/// existed.
pub fn init_vec_table(conn: &Connection) -> Result<bool> {
    if vec_table_exists(conn) {
        debug!("vec_chunks table already exists, skipping creation");
        return Ok(false);
    }
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(
            chunk_id INTEGER PRIMARY KEY,
            embedding float[768]
        )",
    )
    .context("Failed to create vec_chunks table (is sqlite-vec loaded?)")?;
    debug!("vec_chunks virtual table created");
    Ok(true)
}

/// Check whether the `vec_chunks` virtual table exists.
pub fn vec_table_exists(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='vec_chunks'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Vector operations
// ---------------------------------------------------------------------------

/// Insert or replace embedding vectors for the given `(chunk_id, embedding)`
/// pairs.  The embedding slice must be exactly 768 elements.
///
/// Returns the number of rows upserted.
pub fn upsert_vectors(conn: &Connection, vectors: &[(i64, &[f32])]) -> Result<u32> {
    if vectors.is_empty() {
        return Ok(0);
    }
    if !vec_table_exists(conn) {
        anyhow::bail!("vec_chunks table does not exist — sqlite-vec not loaded");
    }

    let mut stmt = conn
        .prepare_cached(
            "INSERT OR REPLACE INTO vec_chunks(chunk_id, embedding) VALUES (?1, ?2)",
        )
        .context("Failed to prepare vec upsert")?;

    let mut count = 0u32;
    for &(chunk_id, embedding) in vectors {
        let blob = vec_to_blob(embedding);
        stmt.execute(rusqlite::params![chunk_id, blob])
            .with_context(|| format!("Failed to upsert vector for chunk_id={chunk_id}"))?;
        count += 1;
    }

    trace!(count, "upserted embedding vectors");
    Ok(count)
}

/// Delete all vectors for chunks belonging to `file_id`.
///
/// Joins through `code_chunks` to find relevant chunk IDs.
/// Returns the number of rows deleted.  Returns 0 when sqlite-vec is not
/// loaded rather than erroring, since the table simply does not exist.
pub fn delete_file_vectors(conn: &Connection, file_id: i64) -> Result<u32> {
    if !vec_table_exists(conn) {
        return Ok(0);
    }

    let deleted = conn
        .execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN (
                SELECT id FROM code_chunks WHERE file_id = ?1
            )",
            [file_id],
        )
        .context("Failed to delete file vectors")?;

    trace!(file_id, deleted, "deleted embedding vectors for file");
    Ok(deleted as u32)
}

/// K-nearest-neighbour search over embedded chunks.
///
/// `query_vector` should be L2-normalised (unit length) when using cosine
/// distance.  Returns `(chunk_id, distance)` pairs ordered by ascending
/// distance (lower = more similar).  Returns an empty vec when sqlite-vec is
/// not loaded.
pub fn knn_search(
    conn: &Connection,
    query_vector: &[f32],
    limit: usize,
) -> Result<Vec<(i64, f64)>> {
    if !vec_table_exists(conn) {
        return Ok(vec![]);
    }

    let blob = vec_to_blob(query_vector);
    let effective_limit = if limit == 0 { 100 } else { limit.min(500) } as i64;

    let mut stmt = conn
        .prepare_cached(
            "SELECT chunk_id, distance
             FROM vec_chunks
             WHERE embedding MATCH ?1
             ORDER BY distance
             LIMIT ?2",
        )
        .context("Failed to prepare KNN query")?;

    let rows = stmt
        .query_map(rusqlite::params![blob, effective_limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })
        .context("KNN query execution failed")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect KNN results")?;

    trace!(result_count = rows.len(), limit, "knn_search completed");
    Ok(rows)
}

/// Count the total number of vectors currently stored.
pub fn vector_count(conn: &Connection) -> Result<u32> {
    if !vec_table_exists(conn) {
        return Ok(0);
    }
    let count: u32 =
        conn.query_row("SELECT COUNT(*) FROM vec_chunks", [], |r| r.get(0))?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Serialisation helpers
// ---------------------------------------------------------------------------

/// Serialise an f32 slice as a little-endian byte blob for sqlite-vec.
pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialise a little-endian byte blob back to `Vec<f32>`.
///
/// Returns an empty vec if the blob length is not a multiple of 4.
pub fn blob_to_vec(blob: &[u8]) -> Vec<f32> {
    if blob.len() % 4 != 0 {
        return vec![];
    }
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "vector_store_tests.rs"]
mod tests;
