use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn incremental_detects_new_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();

    // Full index first.
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();
    // Count only project files — stdlib/ecosystem ecosystems may also
    // populate the `files` table with origin='external' rows, which are
    // not seen by incremental's changeset diff.
    let count1: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // Add a new file.
    fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

    let stats = incremental_index(&mut db, dir.path(), None).unwrap();
    assert_eq!(stats.files_added, 1);
    assert_eq!(stats.files_unchanged, count1);

    let count2: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count2, count1 + 1);
}

#[test]
fn incremental_detects_modified_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Modify the file.
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo { void Bar() {} } }").unwrap();

    let stats = incremental_index(&mut db, dir.path(), None).unwrap();
    assert_eq!(stats.files_modified, 1);
    assert_eq!(stats.files_added, 0);
}

#[test]
fn incremental_detects_deleted_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();
    fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Delete one file.
    fs::remove_file(dir.path().join("b.cs")).unwrap();

    let stats = incremental_index(&mut db, dir.path(), None).unwrap();
    assert_eq!(stats.files_deleted, 1);

    let count: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn incremental_no_changes_is_fast() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let stats = incremental_index(&mut db, dir.path(), None).unwrap();
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.files_modified, 0);
    assert_eq!(stats.files_deleted, 0);
    assert!(stats.files_unchanged > 0);
}

// ------------------------------------------------------------------
// reindex_files tests
// ------------------------------------------------------------------

#[test]
fn reindex_files_handles_single_create() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Add a new file.
    fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "b.cs".to_string(),
        change_kind: ChangeKind::Created,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_added, 1);
    assert_eq!(stats.files_modified, 0);
    assert_eq!(stats.files_deleted, 0);

    let count: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn reindex_files_handles_modify() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Modify the file to add a method.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Foo { void Bar() {} } }",
    )
    .unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "a.cs".to_string(),
        change_kind: ChangeKind::Modified,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_modified, 1);

    // Should have more symbols now (Foo + Bar method).
    let sym_count: u32 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
        .unwrap();
    assert!(sym_count >= 2, "Expected at least Foo + Bar, got {sym_count}");
}

#[test]
fn reindex_files_handles_delete() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();
    fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Delete one file from disk.
    fs::remove_file(dir.path().join("b.cs")).unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "b.cs".to_string(),
        change_kind: ChangeKind::Deleted,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_deleted, 1);

    let count: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn reindex_files_skips_missing_created_file() {
    let dir = TempDir::new().unwrap();
    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Report a created file that doesn't actually exist (race condition).
    let changes = vec![FileChangeEvent {
        relative_path: "phantom.cs".to_string(),
        change_kind: ChangeKind::Created,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.files_modified, 0);
}

#[test]
fn reindex_files_skips_unsupported_extensions() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("image.png"), "binary data").unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "image.png".to_string(),
        change_kind: ChangeKind::Modified,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.files_modified, 0);
}

#[test]
fn reindex_files_empty_changes_is_noop() {
    let dir = TempDir::new().unwrap();
    let mut db = Database::open_in_memory().unwrap();
    let stats = reindex_files(&mut db, dir.path(), &[], None).unwrap();
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.duration_ms, 0);
}

// ------------------------------------------------------------------
// Blast-radius tests
// ------------------------------------------------------------------

/// When file A defines `Foo` and file B calls `Foo`, modifying A should
/// trigger re-resolution of B (blast radius).
#[test]
fn blast_radius_reresolved_on_modify() {
    let dir = TempDir::new().unwrap();

    // File A defines a class with a method.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Svc { public void DoWork() {} } }",
    )
    .unwrap();

    // File B references the method from A.
    fs::write(
        dir.path().join("b.cs"),
        "namespace App { class Consumer { void Run() { DoWork(); } } }",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Verify there's at least one edge from B → A.
    let edge_count_before: u32 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();

    // Modify A: rename the method.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Svc { public void DoWorkRenamed() {} } }",
    )
    .unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "a.cs".to_string(),
        change_kind: ChangeKind::Modified,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_modified, 1);
    // B should be re-resolved via blast radius.
    assert!(
        stats.files_reresolved >= 1,
        "Expected B to be re-resolved, got {}",
        stats.files_reresolved
    );

    // The old edge (B → DoWork) should be gone since DoWork no longer exists.
    // B's reference to DoWork is now unresolvable.
    let unresolved: u32 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
        .unwrap();
    // B's call to DoWork() should now be in unresolved_refs.
    assert!(
        unresolved >= 1,
        "Expected unresolved ref for renamed symbol, got {unresolved} (edges before: {edge_count_before})"
    );
}

/// When a deleted file's symbols are referenced by other files, those
/// dependents should be re-resolved.
#[test]
fn blast_radius_reresolved_on_delete() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Helper { public static void Aid() {} } }",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.cs"),
        "namespace App { class Main { void Go() { Aid(); } } }",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Delete A.
    fs::remove_file(dir.path().join("a.cs")).unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "a.cs".to_string(),
        change_kind: ChangeKind::Deleted,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_deleted, 1);
    assert!(
        stats.files_reresolved >= 1,
        "Expected B to be re-resolved after A was deleted, got {}",
        stats.files_reresolved
    );
}

/// When a new file adds symbols that match previously unresolved refs
/// in other files, those files should be re-resolved.
#[test]
fn blast_radius_resolves_previously_unresolved() {
    let dir = TempDir::new().unwrap();

    // File B references a symbol that doesn't exist yet.
    fs::write(
        dir.path().join("b.cs"),
        "namespace App { class User { void Go() { MissingMethod(); } } }",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Verify that MissingMethod is in unresolved_refs.
    let unresolved_before: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name = 'MissingMethod'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        unresolved_before >= 1,
        "Expected MissingMethod in unresolved_refs"
    );

    // Now create a file that defines MissingMethod.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Lib { public void MissingMethod() {} } }",
    )
    .unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "a.cs".to_string(),
        change_kind: ChangeKind::Created,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_added, 1);
    assert!(
        stats.files_reresolved >= 1,
        "Expected B to be re-resolved when MissingMethod was added, got {}",
        stats.files_reresolved
    );
}

/// No blast radius when the change doesn't affect any dependents.
#[test]
fn blast_radius_zero_when_no_dependents() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Isolated {} }",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.cs"),
        "namespace Other { class Unrelated {} }",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    // Modify A — B has no references to A.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Isolated { void New() {} } }",
    )
    .unwrap();

    let changes = vec![FileChangeEvent {
        relative_path: "a.cs".to_string(),
        change_kind: ChangeKind::Modified,
    }];

    let stats = reindex_files(&mut db, dir.path(), &changes, None).unwrap();
    assert_eq!(stats.files_modified, 1);
    assert_eq!(
        stats.files_reresolved, 0,
        "No files should be re-resolved when there are no dependents"
    );
}
