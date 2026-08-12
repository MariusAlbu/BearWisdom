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
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Foo { void Bar() {} } }",
    )
    .unwrap();

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
    assert!(
        sym_count >= 2,
        "Expected at least Foo + Bar, got {sym_count}"
    );
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

    // File A defines a class with a static method.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Svc { public static void DoWork() {} } }",
    )
    .unwrap();

    // File B references the method from A through its declaring class —
    // a structural `Svc.DoWork` member reference, not a bare name.
    fs::write(
        dir.path().join("b.cs"),
        "namespace App { class Consumer { void Run() { Svc.DoWork(); } } }",
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
        "namespace App { class Svc { public static void DoWorkRenamed() {} } }",
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

/// The Stage 2 win: a body-only edit changes no symbol's key, so every id
/// survives, the inbound edge from the consumer stays valid, and the consumer
/// is NOT re-resolved. Under the old churn-all path B re-resolved on every
/// keystroke; this caps re-resolution to actual contract changes.
#[test]
fn body_only_change_does_not_reresolve_dependents() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Svc { public static void DoWork() {} } }",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.cs"),
        "namespace App { class Consumer { void Run() { Svc.DoWork(); } } }",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let dowork_before: i64 = db
        .conn()
        .query_row("SELECT id FROM symbols WHERE name = 'DoWork'", [], |r| {
            r.get(0)
        })
        .unwrap();

    // Edit only DoWork's body — same name, params, return type ⇒ same key.
    fs::write(
        dir.path().join("a.cs"),
        "namespace App { class Svc { public static void DoWork() { int x = 1; } } }",
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
        "a body-only change must re-resolve no dependents, got {}",
        stats.files_reresolved
    );

    // DoWork kept its id, and B's edge into it survived untouched.
    let dowork_after: i64 = db
        .conn()
        .query_row("SELECT id FROM symbols WHERE name = 'DoWork'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(dowork_after, dowork_before, "survivor kept its id");
    let inbound: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE target_id = ?1",
            [dowork_after],
            |r| r.get(0),
        )
        .unwrap();
    assert!(inbound >= 1, "inbound edge from the consumer is preserved");
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
        "namespace App { class Main { void Go() { Helper.Aid(); } } }",
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

/// When a workspace manifest changes between full and incremental runs, the
/// `packages` table must be refreshed in-place. Earlier behavior only logged
/// a warning, leaving the resolver and `assign_package_ids` blind to the
/// new package until the next full reindex.
#[test]
fn incremental_rewrites_packages_when_manifest_added() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("package.json"),
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("packages/web/src")).unwrap();
    // packages/web needs its own manifest so it counts as a workspace
    // member. Without it, the initial state has zero real members and
    // the root manifest (a workspace controller, not a member) is no
    // longer counted under it.
    fs::write(
        dir.path().join("packages/web/package.json"),
        r#"{"name":"@org/web","version":"0.1.0"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("packages/web/src/index.ts"),
        "export const x = 1;",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let initial: u32 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
        .unwrap();

    // Drop a new package's package.json on disk — this is the manifest change.
    fs::create_dir_all(dir.path().join("packages/api/src")).unwrap();
    fs::write(
        dir.path().join("packages/api/package.json"),
        r#"{"name":"@org/api","version":"0.1.0"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("packages/api/src/server.ts"),
        "export function listen() {}",
    )
    .unwrap();

    incremental_index(&mut db, dir.path(), None).unwrap();

    let after: u32 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
        .unwrap();
    assert!(
        after > initial,
        "manifest change should grow packages table: was {initial}, now {after}",
    );

    let api_present: u32 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE declared_name = '@org/api'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(api_present, 1, "newly added package should be in the table");
}

// ------------------------------------------------------------------
// Derived-qname / structural-scope parity across full vs. incremental
//
// `parent_index` is intra-file and never persisted; both index paths
// rebuild it on every parse and run the same qname normalization. A
// namespace-nested method is the canary: its qualified_name and scope_path
// are derived from the namespace+class parent chain, so they must come out
// byte-identical whether the file was full- or incrementally indexed, and
// must still carry the namespace head rather than collapsing to the
// class-local name.
// ------------------------------------------------------------------

/// Read a method's (qualified_name, scope_path) for the given simple name.
fn read_method_qname(db: &Database, name: &str) -> (String, Option<String>) {
    db.conn()
        .query_row(
            "SELECT qualified_name, scope_path FROM symbols
             WHERE name = ?1 AND kind = 'method' AND origin = 'internal'
             LIMIT 1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn incremental_reproduces_full_index_method_qname_and_scope() {
    let src_v1 = "namespace App { class Svc { void Run() {} } }";
    // Same method, body changed — keeps `Run` present so its derived qname
    // must be reproduced, not freshly invented.
    let src_v2 = "namespace App { class Svc { void Run() { var x = 1; } } }";

    // Reference: what a full index persists for the method.
    let full_dir = TempDir::new().unwrap();
    fs::write(full_dir.path().join("a.cs"), src_v1).unwrap();
    let mut full_db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut full_db, full_dir.path(), None, None, None).unwrap();
    let (full_qn, full_sp) = read_method_qname(&full_db, "Run");

    // The namespace head must survive the structural rebuild.
    assert!(
        full_qn.starts_with("App."),
        "full-index method qname should carry the namespace head: {full_qn}"
    );

    // Same file, then an in-place modification re-indexed incrementally.
    let inc_dir = TempDir::new().unwrap();
    fs::write(inc_dir.path().join("a.cs"), src_v1).unwrap();
    let mut inc_db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut inc_db, inc_dir.path(), None, None, None).unwrap();

    fs::write(inc_dir.path().join("a.cs"), src_v2).unwrap();
    let stats = incremental_index(&mut inc_db, inc_dir.path(), None).unwrap();
    assert_eq!(stats.files_modified, 1, "the .cs file should re-index");

    let (inc_qn, inc_sp) = read_method_qname(&inc_db, "Run");
    assert_eq!(
        inc_qn, full_qn,
        "incremental re-index must reproduce the full-index method qualified_name"
    );
    assert_eq!(
        inc_sp, full_sp,
        "incremental re-index must reproduce the full-index method scope_path"
    );
}

/// An incremental save of a file whose module `use`s a local `__using__`
/// macro must re-run the plugin-state + synthesis phases, not just skip them
/// — before this fix, `synthesize_project_symbols` never ran on the
/// incremental path at all, so a member injected only via `__using__`
/// (invisible to the extractor, no textual trace in the consumer's source)
/// vanished from the DB the moment its file was touched: the survivor-match
/// write drops any symbol the fresh extraction no longer emits, and nothing
/// downstream re-added it.
#[test]
fn incremental_resynthesizes_using_injected_members() {
    let src_v1 = r#"
defmodule Local do
  defmacro __using__(_opts) do
    quote do
      def greet, do: :ok
    end
  end
end

defmodule User do
  use Local
end
"#;
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.ex"), src_v1).unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::indexer::full::full_index(&mut db, dir.path(), None, None, None).unwrap();
    assert!(
        has_symbol(&db, "User.greet"),
        "full index must synthesize the __using__-injected member"
    );

    // Touch the file (no semantic change) so it re-indexes incrementally.
    // The extractor still emits no `greet` for `User` — only the synthesis
    // phase can put it back.
    fs::write(dir.path().join("a.ex"), format!("{src_v1}\n# touch\n")).unwrap();
    let stats = incremental_index(&mut db, dir.path(), None).unwrap();
    assert_eq!(stats.files_modified, 1, "the .ex file should re-index");

    assert!(
        has_symbol(&db, "User.greet"),
        "incremental re-index must re-synthesize the __using__-injected member"
    );
}

fn has_symbol(db: &Database, qualified_name: &str) -> bool {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = ?1",
            [qualified_name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0
}
