// =============================================================================
// search/vector_store.rs  —  sqlite-vec vector storage and KNN search
//
// Manages the `vec_chunks` virtual table (created by the sqlite-vec extension)
// for storing 768-dimensional code embeddings and performing k-nearest-neighbor
// cosine similarity searches.
//
// The sqlite-vec extension must be loaded via `Database::open_with_vec()` before
// any of these functions will work.  If the extension is unavailable, all
// functions return graceful errors or empty results.
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
/// Requires sqlite-vec to be loaded (via `Database::open_with_vec`).
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
            embedding float[768],
            distance_metric = cosine
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
mod tests {
    use super::*;

    fn make_db() -> crate::db::Database {
        crate::db::Database::open_in_memory().unwrap()
    }

    // -----------------------------------------------------------------------
    // vec_to_blob / blob_to_vec — no sqlite-vec required
    // -----------------------------------------------------------------------

    #[test]
    fn vec_to_blob_roundtrip() {
        let original = vec![1.0f32, -2.5, 0.0, 3.14, f32::MIN, f32::MAX];
        let blob = vec_to_blob(&original);
        let recovered = blob_to_vec(&blob);
        assert_eq!(original, recovered);
    }

    #[test]
    fn vec_to_blob_empty() {
        let blob = vec_to_blob(&[]);
        assert!(blob.is_empty());
        assert!(blob_to_vec(&blob).is_empty());
    }

    #[test]
    fn blob_to_vec_wrong_length_returns_empty() {
        let bad = vec![0u8; 7]; // not a multiple of 4
        assert!(blob_to_vec(&bad).is_empty());
    }

    #[test]
    fn vec_to_blob_one_float_correct_bytes() {
        let blob = vec_to_blob(&[1.0f32]);
        // 1.0f32 IEEE 754 LE = 0x3F800000
        assert_eq!(blob, vec![0x00, 0x00, 0x80, 0x3F]);
    }

    // -----------------------------------------------------------------------
    // vec_table_exists / graceful degradation — no sqlite-vec required
    // -----------------------------------------------------------------------

    #[test]
    fn vec_table_not_present_without_extension() {
        let db = make_db();
        assert!(!vec_table_exists(&db.conn));
    }

    #[test]
    fn knn_returns_empty_without_extension() {
        let db = make_db();
        let query = vec![0.0f32; 768];
        let results = knn_search(&db.conn, &query, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn delete_file_vectors_noop_without_extension() {
        let db = make_db();
        let deleted = delete_file_vectors(&db.conn, 99).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn vector_count_zero_without_extension() {
        let db = make_db();
        let count = vector_count(&db.conn).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn upsert_errors_without_extension() {
        let db = make_db();
        let v = vec![0.0f32; 768];
        let result = upsert_vectors(&db.conn, &[(1, v.as_slice())]);
        assert!(result.is_err(), "upsert should fail without sqlite-vec");
    }

    // -----------------------------------------------------------------------
    // Full integration tests — require SQLITE_VEC_PATH env var
    // -----------------------------------------------------------------------

    fn try_load_vec(conn: &Connection) -> bool {
        let path = match std::env::var("SQLITE_VEC_PATH") {
            Ok(p) => p,
            Err(_) => return false,
        };
        if unsafe { conn.load_extension_enable() }.is_err() {
            return false;
        }
        let ok = unsafe { conn.load_extension(&path, None) }.is_ok();
        let _ = conn.load_extension_disable(); // not unsafe in rusqlite 0.33
        ok
    }

    #[test]
    #[ignore]
    fn init_vec_table_creates_table() {
        let db = make_db();
        if !try_load_vec(&db.conn) {
            eprintln!("Skipping: SQLITE_VEC_PATH not set");
            return;
        }

        let created = init_vec_table(&db.conn).unwrap();
        assert!(created, "Should have created the table");
        assert!(vec_table_exists(&db.conn));

        // Idempotent second call.
        assert!(!init_vec_table(&db.conn).unwrap());
    }

    #[test]
    #[ignore]
    fn upsert_and_knn_search_roundtrip() {
        let db = make_db();
        if !try_load_vec(&db.conn) {
            return;
        }
        init_vec_table(&db.conn).unwrap();

        let mut v1 = vec![0.0f32; 768];
        v1[0] = 1.0;
        let mut v2 = vec![0.0f32; 768];
        v2[1] = 1.0;

        upsert_vectors(&db.conn, &[(1, &v1), (2, &v2)]).unwrap();
        assert_eq!(vector_count(&db.conn).unwrap(), 2);

        let results = knn_search(&db.conn, &v1, 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 1, "Nearest chunk should be chunk_id 1");
    }

    #[test]
    #[ignore]
    fn delete_file_vectors_removes_rows_via_code_chunks() {
        let db = make_db();
        if !try_load_vec(&db.conn) {
            return;
        }
        init_vec_table(&db.conn).unwrap();

        db.conn
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed) VALUES ('f.rs', 'h', 'rust', 0)",
                [],
            )
            .unwrap();
        let file_id: i64 = db.conn.last_insert_rowid();

        db.conn
            .execute(
                "INSERT INTO code_chunks (file_id, content_hash, content, start_line, end_line)
                 VALUES (?1, 'x', 'fn f(){}', 0, 0)",
                rusqlite::params![file_id],
            )
            .unwrap();
        let chunk_id: i64 = db.conn.last_insert_rowid();

        let v = vec![0.0f32; 768];
        upsert_vectors(&db.conn, &[(chunk_id, &v)]).unwrap();
        assert_eq!(vector_count(&db.conn).unwrap(), 1);

        let deleted = delete_file_vectors(&db.conn, file_id).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(vector_count(&db.conn).unwrap(), 0);
    }
}
