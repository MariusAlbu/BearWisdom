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
            .prepare("SELECT id, path FROM files")
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Insert a row into `files` and return its id.
    fn insert_file(conn: &Connection, path: &str, language: &str) -> i64 {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'hash', ?2, 0)",
            rusqlite::params![path, language],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn fts_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM fts_content", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn index_single_file_inserts_row() {
        let db = Database::open_in_memory().unwrap();
        let id = insert_file(&db.conn, "src/main.rs", "rust");

        index_file_content(&db.conn, id, "src/main.rs", "fn main() {}").unwrap();

        assert_eq!(fts_count(&db.conn), 1);
    }

    #[test]
    fn index_file_replaces_existing_entry() {
        let db = Database::open_in_memory().unwrap();
        let id = insert_file(&db.conn, "a.rs", "rust");

        index_file_content(&db.conn, id, "a.rs", "version one").unwrap();
        index_file_content(&db.conn, id, "a.rs", "version two").unwrap();

        // Still only one row — no duplicates.
        assert_eq!(fts_count(&db.conn), 1);

        // The trigram index should match the new content.
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM fts_content WHERE fts_content MATCH 'two'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn remove_file_content_deletes_row() {
        let db = Database::open_in_memory().unwrap();
        let id = insert_file(&db.conn, "b.rs", "rust");
        index_file_content(&db.conn, id, "b.rs", "some content").unwrap();

        assert_eq!(fts_count(&db.conn), 1);
        remove_file_content(&db.conn, id).unwrap();
        assert_eq!(fts_count(&db.conn), 0);
    }

    #[test]
    fn batch_index_content_returns_count() {
        let db = Database::open_in_memory().unwrap();
        let id1 = insert_file(&db.conn, "f1.ts", "typescript");
        let id2 = insert_file(&db.conn, "f2.ts", "typescript");
        let id3 = insert_file(&db.conn, "f3.ts", "typescript");

        let files = vec![
            (id1, "f1.ts", "const x = 1;"),
            (id2, "f2.ts", "const y = 2;"),
            (id3, "f3.ts", "const z = 3;"),
        ];
        let count = batch_index_content(&db.conn, &files).unwrap();

        assert_eq!(count, 3);
        assert_eq!(fts_count(&db.conn), 3);
    }

    #[test]
    fn batch_index_content_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let id = insert_file(&db.conn, "dup.rs", "rust");

        let files = vec![(id, "dup.rs", "fn foo() {}")];
        batch_index_content(&db.conn, &files).unwrap();
        batch_index_content(&db.conn, &files).unwrap();

        // Re-indexing the same file should not create duplicates.
        assert_eq!(fts_count(&db.conn), 1);
    }

    #[test]
    fn batch_empty_slice_returns_zero() {
        let db = Database::open_in_memory().unwrap();
        let count = batch_index_content(&db.conn, &[]).unwrap();
        assert_eq!(count, 0);
        assert_eq!(fts_count(&db.conn), 0);
    }

    #[test]
    fn rebuild_reads_files_from_disk() {
        use std::io::Write;
        use tempfile::TempDir;

        let root = TempDir::new().unwrap();
        let db = Database::open_in_memory().unwrap();

        // Write a real file to disk and register it in `files`.
        let rel = "hello.rs";
        let abs = root.path().join(rel);
        let mut f = std::fs::File::create(&abs).unwrap();
        f.write_all(b"fn hello() {}").unwrap();

        let id = insert_file(&db.conn, rel, "rust");

        let count = rebuild_content_index(&db.conn, root.path()).unwrap();
        assert_eq!(count, 1);

        // Trigram search should find content from the file.
        let found: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM fts_content WHERE fts_content MATCH 'hello'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);

        // File id should be the rowid.
        let rowid: i64 = db
            .conn
            .query_row("SELECT rowid FROM fts_content", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rowid, id);
    }

    #[test]
    fn rebuild_skips_missing_files_gracefully() {
        let root = tempfile::TempDir::new().unwrap();
        let db = Database::open_in_memory().unwrap();

        // Register a file that doesn't exist on disk.
        insert_file(&db.conn, "ghost.rs", "rust");

        // Should not error — just returns 0 indexed.
        let count = rebuild_content_index(&db.conn, root.path()).unwrap();
        assert_eq!(count, 0);
        assert_eq!(fts_count(&db.conn), 0);
    }
}
