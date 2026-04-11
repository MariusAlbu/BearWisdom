// =============================================================================
// search/content_index.rs  —  FTS5 trigram content index management
//
// Maintains the `fts_content` virtual table, which provides fast substring
// search across all indexed file content via SQLite's trigram tokenizer.
//
// Design notes:
//   • `fts_content` is a contentless FTS5 table (`content = ''`).  SQLite
//     stores only the trigram index, not the original text.
//   • Deletion requires the original text in standard FTS5, but
//     `contentless_delete = 1` (SQLite 3.43+) lifts that restriction.
//   • We use the file's `id` (INTEGER PRIMARY KEY) as the FTS rowid so
//     joins back to the `files` table are a trivial rowid lookup.
//   • `batch_index_content` wraps all inserts in a single transaction for
//     throughput.  For single-file updates the overhead is acceptable.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Index a single file's content into the FTS5 trigram table.
///
/// Uses `file_id` as the FTS rowid.  Any existing entry for that rowid is
/// removed first so re-indexing a modified file produces a clean state.
pub fn index_file_content(
    conn: &Connection,
    file_id: i64,
    path: &str,
    content: &str,
) -> Result<()> {
    // Remove stale entry (no-op if absent; contentless_delete = 1 means
    // we do not need the original text to perform the delete).
    remove_file_content(conn, file_id)?;

    conn.execute(
        "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
        rusqlite::params![file_id, path, content],
    )
    .with_context(|| format!("Failed to insert FTS content for file_id={file_id} path={path}"))?;

    debug!(file_id, path, "indexed into fts_content");
    Ok(())
}

/// Batch-insert file contents into the trigram index.
///
/// `files` is a slice of `(file_id, path, content)` tuples.
/// All operations run inside a single explicit transaction for throughput.
/// Returns the number of files successfully indexed.
pub fn batch_index_content(conn: &Connection, files: &[(i64, &str, &str)]) -> Result<u32> {
    if files.is_empty() {
        return Ok(0);
    }

    conn.execute_batch("BEGIN")
        .context("Failed to begin transaction for batch FTS index")?;

    let mut count = 0u32;

    for &(file_id, path, content) in files {
        // Delete before insert — idempotent for re-indexing.
        if let Err(e) = conn.execute(
            "DELETE FROM fts_content WHERE rowid = ?1",
            [file_id],
        ) {
            warn!(file_id, path, "FTS delete failed: {e}");
            continue;
        }

        match conn.execute(
            "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![file_id, path, content],
        ) {
            Ok(_) => count += 1,
            Err(e) => warn!(file_id, path, "FTS insert failed: {e}"),
        }
    }

    conn.execute_batch("COMMIT")
        .context("Failed to commit batch FTS index")?;

    debug!(count, "batch_index_content complete");
    Ok(count)
}

/// Remove a single file from the trigram content index.
pub fn remove_file_content(conn: &Connection, file_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM fts_content WHERE rowid = ?1",
        [file_id],
    )
    .with_context(|| format!("Failed to delete FTS content for file_id={file_id}"))?;
    Ok(())
}

/// Rebuild the entire FTS5 content index from the `files` table.
///
/// Reads every file referenced in `files` from disk (resolving relative paths
/// against `project_root`), clears the old index, and re-inserts everything.
/// Returns the number of files successfully indexed.
pub fn rebuild_content_index(conn: &Connection, project_root: &Path) -> Result<u32> {
    // Collect (id, path) rows first so we hold no statement borrow during I/O.
    let file_rows: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE origin = 'internal'")
            .context("Failed to prepare files query for rebuild")?;
        let iter = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query files for rebuild")?;
        iter.filter_map(|r| match r {
            Ok(row) => Some(row),
            Err(e) => {
                warn!("Row error during rebuild query: {e}");
                None
            }
        })
        .collect()
    };

    // Clear the existing index in one shot, then re-populate inside a transaction.
    conn.execute_batch("DELETE FROM fts_content")
        .context("Failed to clear fts_content for rebuild")?;

    conn.execute_batch("BEGIN")
        .context("Failed to begin transaction for rebuild")?;

    let mut count = 0u32;

    for (file_id, rel_path) in &file_rows {
        let abs_path = project_root.join(rel_path.replace('\\', "/").trim_start_matches('/'));

        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(file_id, path = %rel_path, "Cannot read file for FTS rebuild: {e}");
                continue;
            }
        };

        match conn.execute(
            "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![file_id, rel_path, content],
        ) {
            Ok(_) => count += 1,
            Err(e) => warn!(file_id, path = %rel_path, "FTS insert failed during rebuild: {e}"),
        }
    }

    conn.execute_batch("COMMIT")
        .context("Failed to commit FTS rebuild")?;

    debug!(count, "rebuild_content_index complete");
    Ok(count)
}

/// `Database`-accepting wrapper for [`rebuild_content_index`].
///
/// Convenience entry point for callers that hold a `&Database` rather than
/// a `&Connection`.  Delegates to the free function via the `conn()` accessor.
pub fn rebuild_content_index_db(db: &crate::db::Database, project_root: &Path) -> Result<u32> {
    rebuild_content_index(db.conn(), project_root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "content_index_tests.rs"]
mod tests;
