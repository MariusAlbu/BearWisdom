use super::*;
use crate::db::Database;
use crate::query::ref_snapshot::export_ref_snapshot;

/// A same-file call whose target is initially undefined logs as `unresolved`;
/// defining the target and reindexing flips exactly that ref to `resolved`.
#[test]
fn diff_detects_a_seeded_flip_from_unresolved_to_resolved() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_path = dir.path().join("lib.ts");
    std::fs::write(
        &src_path,
        "export function run() {\n  return missingHelper();\n}\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();
    let old = export_ref_snapshot(&db).unwrap();

    let seeded = old
        .iter()
        .find(|e| e.target == "missingHelper" && e.kind == "calls")
        .expect("the missing call must be logged as unresolved");
    assert_eq!(seeded.outcome, "unresolved");

    // Mutate: define the previously-missing function, flipping the ref.
    std::fs::write(
        &src_path,
        "export function run() {\n  return missingHelper();\n}\n\nexport function missingHelper() {\n  return 42;\n}\n",
    )
    .unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let report = diff_against_db(&db, &old, 20).unwrap();
    assert_eq!(
        report.newly_resolved.count, 1,
        "exactly one ref must flip to resolved"
    );
    assert_eq!(report.newly_unresolved.count, 0);
    assert_eq!(report.retargeted.count, 0);
    assert_eq!(report.drain_transitions.count, 0);

    let flip = &report.newly_resolved.samples[0];
    assert_eq!(flip.target, "missingHelper");
    assert_eq!(flip.kind, "calls");
    assert_eq!(flip.old_outcome, "unresolved");
    assert_eq!(flip.new_outcome, "resolved");
}

// ---------------------------------------------------------------------------
// Retargeting: same ref-site key, different resolved target between snapshots.
// ---------------------------------------------------------------------------

fn insert_file(db: &Database, path: &str, lang: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', ?2, 0)",
            rusqlite::params![path, lang],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn insert_symbol(db: &Database, file_id: i64, name: &str, qname: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
             VALUES (?1, ?2, ?3, 'function', 1, 0)",
            rusqlite::params![file_id, name, qname],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

#[allow(clippy::too_many_arguments)]
fn insert_ref_resolution(
    db: &Database,
    source_id: i64,
    target_name: &str,
    kind: &str,
    line: u32,
    col: u32,
    outcome: &str,
    target_id: Option<i64>,
) {
    db.conn()
        .execute(
            "INSERT INTO ref_resolutions \
             (source_id, target_name, kind, source_line, source_col, outcome, target_id, confidence, strategy) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1.0, 'import_binding')",
            rusqlite::params![source_id, target_name, kind, line, col, outcome, target_id],
        )
        .unwrap();
}

#[test]
fn diff_detects_retargeting_when_the_same_ref_site_binds_a_different_candidate() {
    let db = Database::open_in_memory().unwrap();

    let src_file = insert_file(&db, "src/x.ts", "typescript");
    let a_file = insert_file(&db, "src/a.ts", "typescript");
    let b_file = insert_file(&db, "src/b.ts", "typescript");

    let source_sym = insert_symbol(&db, src_file, "caller", "caller");
    let target_a = insert_symbol(&db, a_file, "thing", "A.thing");
    let target_b = insert_symbol(&db, b_file, "thing", "B.thing");

    insert_ref_resolution(&db, source_sym, "thing", "calls", 5, 2, "resolved", Some(target_a));
    let old = export_ref_snapshot(&db).unwrap();

    db.conn().execute("DELETE FROM ref_resolutions", []).unwrap();
    insert_ref_resolution(&db, source_sym, "thing", "calls", 5, 2, "resolved", Some(target_b));
    let new = export_ref_snapshot(&db).unwrap();

    let report = diff_snapshots(&old, &new, 20);
    assert_eq!(report.retargeted.count, 1, "the winner must be flagged as retargeted");
    assert_eq!(report.newly_resolved.count, 0);
    assert_eq!(report.newly_unresolved.count, 0);
    assert_eq!(report.drain_transitions.count, 0);

    let flip = &report.retargeted.samples[0];
    assert_eq!(flip.old_target_qname.as_deref(), Some("A.thing"));
    assert_eq!(flip.new_target_qname.as_deref(), Some("B.thing"));
}
