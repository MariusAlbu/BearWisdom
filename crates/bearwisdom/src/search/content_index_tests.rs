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
    let id = insert_file(db.conn(), "src/main.rs", "rust");

    index_file_content(db.conn(), id, "src/main.rs", "fn main() {}").unwrap();

    assert_eq!(fts_count(db.conn()), 1);
}

#[test]
fn index_file_replaces_existing_entry() {
    let db = Database::open_in_memory().unwrap();
    let id = insert_file(db.conn(), "a.rs", "rust");

    index_file_content(db.conn(), id, "a.rs", "version one").unwrap();
    index_file_content(db.conn(), id, "a.rs", "version two").unwrap();

    // Still only one row — no duplicates.
    assert_eq!(fts_count(db.conn()), 1);

    // The trigram index should match the new content.
    let count: i64 = db
        .conn()
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
    let id = insert_file(db.conn(), "b.rs", "rust");
    index_file_content(db.conn(), id, "b.rs", "some content").unwrap();

    assert_eq!(fts_count(db.conn()), 1);
    remove_file_content(db.conn(), id).unwrap();
    assert_eq!(fts_count(db.conn()), 0);
}

#[test]
fn batch_index_content_returns_count() {
    let db = Database::open_in_memory().unwrap();
    let id1 = insert_file(db.conn(), "f1.ts", "typescript");
    let id2 = insert_file(db.conn(), "f2.ts", "typescript");
    let id3 = insert_file(db.conn(), "f3.ts", "typescript");

    let files = vec![
        (id1, "f1.ts", "const x = 1;"),
        (id2, "f2.ts", "const y = 2;"),
        (id3, "f3.ts", "const z = 3;"),
    ];
    let count = batch_index_content(db.conn(), &files).unwrap();

    assert_eq!(count, 3);
    assert_eq!(fts_count(db.conn()), 3);
}

#[test]
fn batch_index_content_is_idempotent() {
    let db = Database::open_in_memory().unwrap();
    let id = insert_file(db.conn(), "dup.rs", "rust");

    let files = vec![(id, "dup.rs", "fn foo() {}")];
    batch_index_content(db.conn(), &files).unwrap();
    batch_index_content(db.conn(), &files).unwrap();

    // Re-indexing the same file should not create duplicates.
    assert_eq!(fts_count(db.conn()), 1);
}

#[test]
fn batch_empty_slice_returns_zero() {
    let db = Database::open_in_memory().unwrap();
    let count = batch_index_content(db.conn(), &[]).unwrap();
    assert_eq!(count, 0);
    assert_eq!(fts_count(db.conn()), 0);
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

    let id = insert_file(db.conn(), rel, "rust");

    let count = rebuild_content_index(db.conn(), root.path()).unwrap();
    assert_eq!(count, 1);

    // Trigram search should find content from the file.
    let found: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM fts_content WHERE fts_content MATCH 'hello'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(found, 1);

    // File id should be the rowid.
    let rowid: i64 = db
        .conn()
        .query_row("SELECT rowid FROM fts_content", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rowid, id);
}

#[test]
fn rebuild_skips_missing_files_gracefully() {
    let root = tempfile::TempDir::new().unwrap();
    let db = Database::open_in_memory().unwrap();

    // Register a file that doesn't exist on disk.
    insert_file(db.conn(), "ghost.rs", "rust");

    // Should not error — just returns 0 indexed.
    let count = rebuild_content_index(db.conn(), root.path()).unwrap();
    assert_eq!(count, 0);
    assert_eq!(fts_count(db.conn()), 0);
}
