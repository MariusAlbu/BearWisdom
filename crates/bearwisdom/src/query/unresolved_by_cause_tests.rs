// Sibling test file for `unresolved_by_cause.rs`.

use super::*;
use crate::db::Database;

fn open() -> Database {
    Database::open_in_memory().unwrap()
}

fn seed_file(db: &Database, path: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin)
             VALUES (?1, 'h', 'typescript', 0, 'internal')",
            rusqlite::params![path],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn seed_symbol(db: &Database, file_id: i64, name: &str, line: i64) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, ?2, ?3, 'function', ?4, 0)",
            rusqlite::params![file_id, name, format!("mod.{name}"), line],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

#[allow(clippy::too_many_arguments)]
fn seed_unresolved(
    db: &Database,
    source_id: i64,
    target_name: &str,
    line: i64,
    cause_symbol_id: Option<i64>,
    cause_kind: Option<&str>,
) {
    db.conn()
        .execute(
            "INSERT INTO unresolved_refs
                 (source_id, target_name, kind, source_line, cause_symbol_id, cause_kind)
             VALUES (?1, ?2, 'calls', ?3, ?4, ?5)",
            rusqlite::params![source_id, target_name, line, cause_symbol_id, cause_kind],
        )
        .unwrap();
}

/// N member-refs on a root whose initializer's return was never captured all
/// group under ONE cause: the initializer, not the root binding itself — the
/// CLAUDE.md worked example (`createScopedLogger` → `logger.info` cascade).
#[test]
fn member_refs_on_uncaptured_initializer_group_under_one_cause() {
    let db = open();
    let file = seed_file(&db, "src/logger.ts");
    let factory = seed_symbol(&db, file, "createScopedLogger", 1);
    let caller = seed_symbol(&db, file, "caller", 10);

    for (i, member) in ["info", "warn", "error"].iter().enumerate() {
        seed_unresolved(
            &db,
            caller,
            member,
            10 + i as i64,
            Some(factory),
            Some("uncaptured_return"),
        );
    }

    let report = unresolved_by_cause(&db, 10, 10).unwrap();
    assert_eq!(report.total_caused, 3);
    assert_eq!(report.total_uncaused, 0);
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(group.cause_kind, "uncaptured_return");
    assert_eq!(group.cause_symbol_id, Some(factory));
    assert_eq!(group.cause_qualified_name.as_deref(), Some("mod.createScopedLogger"));
    assert_eq!(group.ref_count, 3);
    assert_eq!(group.samples.len(), 3);
}

/// A member genuinely absent from an internal type groups under
/// `member_missing`, distinct from an uncaptured-return cause on a different
/// symbol even when both die inside the same source file.
#[test]
fn member_missing_groups_separately_from_uncaptured_return() {
    let db = open();
    let file = seed_file(&db, "src/mixed.ts");
    let factory = seed_symbol(&db, file, "makeThing", 1);
    let receiver_type = seed_symbol(&db, file, "Thing", 2);
    let caller = seed_symbol(&db, file, "caller", 10);

    seed_unresolved(&db, caller, "value", 10, Some(factory), Some("uncaptured_return"));
    seed_unresolved(&db, caller, "missingMethod", 11, Some(receiver_type), Some("member_missing"));

    let report = unresolved_by_cause(&db, 10, 10).unwrap();
    assert_eq!(report.total_caused, 2);
    assert_eq!(report.groups.len(), 2);
    let kinds: std::collections::HashSet<&str> =
        report.groups.iter().map(|g| g.cause_kind.as_str()).collect();
    assert!(kinds.contains("uncaptured_return"));
    assert!(kinds.contains("member_missing"));
}

/// `unbound_root` carries no cause symbol; distinct target names must not be
/// lumped into one undifferentiated group.
#[test]
fn unbound_root_groups_by_target_name_when_symbol_less() {
    let db = open();
    let file = seed_file(&db, "src/free.ts");
    let caller = seed_symbol(&db, file, "caller", 10);

    seed_unresolved(&db, caller, "unknownA", 10, None, Some("unbound_root"));
    seed_unresolved(&db, caller, "unknownA", 11, None, Some("unbound_root"));
    seed_unresolved(&db, caller, "unknownB", 12, None, Some("unbound_root"));

    let report = unresolved_by_cause(&db, 10, 10).unwrap();
    assert_eq!(report.total_caused, 3);
    assert_eq!(report.groups.len(), 2, "unknownA and unknownB must be separate groups");
    let a = report
        .groups
        .iter()
        .find(|g| g.samples.iter().any(|s| s.target_name == "unknownA"))
        .unwrap();
    assert_eq!(a.ref_count, 2);
    assert_eq!(a.cause_symbol_id, None);
}

/// A row with no recorded cause (death site outside the instrumented paths)
/// counts toward `total_uncaused`, not toward any group.
#[test]
fn uncaused_rows_are_counted_but_not_grouped() {
    let db = open();
    let file = seed_file(&db, "src/plain.ts");
    let caller = seed_symbol(&db, file, "caller", 10);

    seed_unresolved(&db, caller, "noCause", 10, None, None);

    let report = unresolved_by_cause(&db, 10, 10).unwrap();
    assert_eq!(report.total_caused, 0);
    assert_eq!(report.total_uncaused, 1);
    assert!(report.groups.is_empty());
}

/// Groups are ranked largest-cascade-first, and `top_n` caps the result to
/// the largest groups.
#[test]
fn groups_ranked_by_ref_count_desc_and_capped_by_top_n() {
    let db = open();
    let file = seed_file(&db, "src/rank.ts");
    let big_cause = seed_symbol(&db, file, "bigFactory", 1);
    let small_cause = seed_symbol(&db, file, "smallFactory", 2);
    let caller = seed_symbol(&db, file, "caller", 10);

    for i in 0..5 {
        seed_unresolved(&db, caller, "m", 10 + i, Some(big_cause), Some("uncaptured_return"));
    }
    seed_unresolved(&db, caller, "m", 20, Some(small_cause), Some("uncaptured_return"));

    let report = unresolved_by_cause(&db, 1, 10).unwrap();
    assert_eq!(report.groups.len(), 1, "top_n=1 must cap to the single largest group");
    assert_eq!(report.groups[0].cause_symbol_id, Some(big_cause));
    assert_eq!(report.groups[0].ref_count, 5);
}

/// `samples_per_group` caps the sample list without affecting `ref_count`.
#[test]
fn samples_capped_independently_of_ref_count() {
    let db = open();
    let file = seed_file(&db, "src/samples.ts");
    let cause = seed_symbol(&db, file, "factory", 1);
    let caller = seed_symbol(&db, file, "caller", 10);

    for i in 0..8 {
        seed_unresolved(&db, caller, "m", 10 + i, Some(cause), Some("uncaptured_return"));
    }

    let report = unresolved_by_cause(&db, 10, 3).unwrap();
    assert_eq!(report.groups[0].ref_count, 8);
    assert_eq!(report.groups[0].samples.len(), 3);
}
