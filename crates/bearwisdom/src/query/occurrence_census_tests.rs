use super::*;
use crate::occurrence::Disposition;

fn file(db: &Database, id: i64, path: &str, language: &str) {
    db.conn().execute(
        "INSERT INTO files (id, path, hash, language, last_indexed) VALUES (?1, ?2, 'current', ?3, 0)",
        rusqlite::params![id, path, language],
    ).unwrap();
}

fn measure(db: &Database, id: i64, hash: &str, buckets: &[OccurrenceBucket]) {
    db.conn()
        .execute(
            "INSERT INTO resolution_census VALUES (?1, ?2, ?3)",
            rusqlite::params![id, hash, serde_json::to_string(buckets).unwrap()],
        )
        .unwrap();
}

fn bucket(language: &str, disposition: Disposition, count: u64) -> OccurrenceBucket {
    OccurrenceBucket {
        language: language.into(),
        kind: EdgeKind::Calls,
        from_snippet: false,
        disposition,
        count,
    }
}

#[test]
fn unmeasured_stale_empty_and_legacy_are_not_success() {
    let db = Database::open_in_memory().unwrap();
    assert_eq!(
        occurrence_census(&db).unwrap().binding_coverage_percent,
        None
    );
    file(&db, 1, "one.ts", "typescript");
    file(&db, 2, "two.ts", "typescript");
    measure(
        &db,
        1,
        "old",
        &[bucket("typescript", Disposition::Resolved, 100)],
    );
    let report = occurrence_census(&db).unwrap();
    assert_eq!(
        (report.missing_files, report.stale_files, report.raw.total()),
        (1, 1, 0)
    );
    assert_eq!(report.binding_coverage_percent, None);
    db.conn()
        .execute("DROP TABLE resolution_census", [])
        .unwrap();
    let legacy = occurrence_census(&db).unwrap();
    assert_eq!((legacy.internal_files, legacy.missing_files), (2, 2));
    assert_eq!(legacy.binding_coverage_percent, None);
}

#[test]
fn skips_remain_in_the_denominator_and_external_sources_do_not_inflate_it() {
    let db = Database::open_in_memory().unwrap();
    file(&db, 1, "main.ts", "typescript");
    file(&db, 2, "ext:dependency.ts", "typescript");
    db.conn()
        .execute("UPDATE files SET origin = 'external' WHERE id = 2", [])
        .unwrap();
    measure(
        &db,
        1,
        "current",
        &[
            bucket("typescript", Disposition::Resolved, 4),
            bucket("typescript", Disposition::Unresolved, 1),
            bucket("typescript", Disposition::MissingSourceSymbol, 3),
            bucket("typescript", Disposition::Drained, 20),
            bucket("typescript", Disposition::Duplicate, 2),
        ],
    );
    measure(
        &db,
        2,
        "current",
        &[bucket("typescript", Disposition::Resolved, 1000)],
    );
    let report = occurrence_census(&db).unwrap();
    assert_eq!(report.internal_files, 1);
    assert_eq!(report.raw.total(), 30);
    assert_eq!(report.binding_coverage_percent, Some(50.0));
    assert_eq!(report.binding_precision_percent, None);
    assert_eq!(report.correct_binding_recall_percent, None);
}

#[test]
fn exclusions_are_symmetric_disjoint_and_embedded_languages_are_retained() {
    let db = Database::open_in_memory().unwrap();
    file(&db, 1, "view.vue", "vue");
    file(&db, 2, "model.g.dart", "dart");
    file(&db, 3, "README.md", "markdown");
    let mut snippet = bucket("typescript", Disposition::Resolved, 10);
    snippet.from_snippet = true;
    let mut snippet_miss = snippet.clone();
    snippet_miss.disposition = Disposition::Unresolved;
    measure(
        &db,
        1,
        "current",
        &[
            snippet,
            snippet_miss,
            bucket("typescript", Disposition::Resolved, 3),
            bucket("typescript", Disposition::Unresolved, 1),
        ],
    );
    measure(
        &db,
        2,
        "current",
        &[
            bucket("dart", Disposition::Resolved, 9),
            bucket("dart", Disposition::Unresolved, 7),
        ],
    );
    let mut doc = bucket("markdown", Disposition::Resolved, 6);
    doc.kind = EdgeKind::Imports;
    let mut doc_miss = doc.clone();
    doc_miss.disposition = Disposition::Unresolved;
    measure(&db, 3, "current", &[doc, doc_miss]);
    let report = occurrence_census(&db).unwrap();
    assert_eq!(report.raw.total(), 52);
    assert_eq!(report.code.total(), 4);
    assert_eq!(
        (
            report.excluded_snippets,
            report.excluded_generated,
            report.excluded_document_imports
        ),
        (20, 16, 12)
    );
    assert_eq!(report.by_language_kind["typescript.calls"].resolved, 3);
    assert_eq!(report.binding_coverage_percent, Some(75.0));
}

#[test]
fn invalid_persistence_is_an_error_not_a_silent_zero() {
    let db = Database::open_in_memory().unwrap();
    file(&db, 1, "main.ts", "typescript");
    db.conn()
        .execute(
            "INSERT INTO resolution_census VALUES (1, 'current', 'invalid')",
            [],
        )
        .unwrap();
    assert!(occurrence_census(&db).is_err());
}

#[test]
fn real_pipeline_counts_repeated_calls_by_occurrence_not_edge() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.py"),
        "def helper():\n    pass\n\ndef caller():\n    helper(); helper()\n    helper()\n    helper()\n"
    ).unwrap();
    let mut db = Database::open_in_memory().unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();
    let report = occurrence_census(&db).unwrap();
    assert_eq!(
        report.missing_files, 0,
        "every internal file needs a census, including zero-ref files"
    );
    assert_eq!(report.by_language_kind["python.calls"].resolved, 4);
    let edges: u64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM edges WHERE kind = 'calls'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        edges, 3,
        "legacy graph still deduplicates two calls on the same line"
    );
}

#[test]
fn incremental_replacement_zero_refs_and_deletion_match_full_census() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("main.py");
    let untouched = dir.path().join("untouched.py");
    std::fs::write(
        &path,
        "def helper():\n    pass\n\ndef caller():\n    helper(); helper()\n",
    )
    .unwrap();
    std::fs::write(&untouched, "def other():\n    missing_xyz()\n").unwrap();
    let mut db = Database::open_in_memory().unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();
    let initial = occurrence_census(&db).unwrap();
    assert_eq!(initial.by_language_kind["python.calls"].resolved, 2);

    std::fs::write(
        &path,
        "def helper():\n    pass\n\ndef caller():\n    helper()\n",
    )
    .unwrap();
    crate::incremental_index(&mut db, dir.path(), None).unwrap();
    let incremental = occurrence_census(&db).unwrap();
    assert_eq!(incremental.missing_files + incremental.stale_files, 0);
    assert_eq!(incremental.by_language_kind["python.calls"].resolved, 1);
    assert_eq!(
        incremental.by_language_kind["python.calls"].unresolved, 1,
        "untouched census must survive"
    );
    let mut fresh = Database::open_in_memory().unwrap();
    crate::full_index(&mut fresh, dir.path(), None, None, None).unwrap();
    assert_eq!(
        serde_json::to_value(&incremental).unwrap(),
        serde_json::to_value(occurrence_census(&fresh).unwrap()).unwrap()
    );

    std::fs::write(&path, "# no references or declarations remain\n").unwrap();
    crate::incremental_index(&mut db, dir.path(), None).unwrap();
    let empty = occurrence_census(&db).unwrap();
    assert_eq!(empty.measured_files, 2);
    assert_eq!(
        empty.code.resolved, 0,
        "zero-ref replacement must erase old counts"
    );

    std::fs::remove_file(&untouched).unwrap();
    crate::incremental_index(&mut db, dir.path(), None).unwrap();
    let deleted = occurrence_census(&db).unwrap();
    assert_eq!(
        (
            deleted.internal_files,
            deleted.measured_files,
            deleted.code.total()
        ),
        (1, 1, 0)
    );
    assert_eq!(deleted.binding_coverage_percent, None);
}

#[test]
fn repeated_full_index_replaces_census_instead_of_accumulating() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def caller():\n    missing_xyz()\n",
    )
    .unwrap();
    let mut db = Database::open_in_memory().unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();
    let before = occurrence_census(&db).unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();
    let after = occurrence_census(&db).unwrap();
    assert!(before.raw.total() > 0);
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}
