// Sibling test file for `diagnostics.rs`.

use super::*;
use crate::db::Database;

#[test]
fn test_empty_file_returns_empty_diagnostics() {
    let db = Database::open_in_memory().unwrap();
    let result = get_diagnostics(&db, "nonexistent.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(result.unresolved_count, 0);
    assert_eq!(result.low_confidence_count, 0);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn test_unresolved_refs_surfaced() {
    let db = Database::open_in_memory().unwrap();
    db.conn().execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
        [],
    ).unwrap();
    let file_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'foo', 'mod::foo', 'function', 5, 0)",
            [file_id],
        )
        .unwrap();
    let sym_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line)
         VALUES (?1, 'Bar', 'type_ref', 8)",
            [sym_id],
        )
        .unwrap();

    let result = get_diagnostics(&db, "src/a.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(result.unresolved_count, 1);
    assert_eq!(result.diagnostics[0].kind, DiagnosticKind::UnresolvedSymbol);
    assert_eq!(result.diagnostics[0].line, 8);
    assert_eq!(result.diagnostics[0].target_name.as_deref(), Some("Bar"));
}

#[test]
fn test_drained_ref_is_not_surfaced_as_a_diagnostic() {
    // A ref a rule drained (BuiltinSkipRule) is a known language construct,
    // not a code issue — it must not appear as a squiggle.
    let db = Database::open_in_memory().unwrap();
    db.conn().execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('script.sh', 'h', 'bash', 0)",
        [],
    ).unwrap();
    let file_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'caller', 'mod::caller', 'function', 1, 0)",
            [file_id],
        )
        .unwrap();
    let sym_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line, drained)
         VALUES (?1, 'echo', 'calls', 3, 1)",
            [sym_id],
        )
        .unwrap();

    let result = get_diagnostics(&db, "script.sh", LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(result.unresolved_count, 0);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn test_low_confidence_edges_surfaced() {
    let db = Database::open_in_memory().unwrap();
    db.conn().execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
        [],
    ).unwrap();
    let file_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'caller', 'mod::caller', 'function', 1, 0)",
            [file_id],
        )
        .unwrap();
    let src_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'callee', 'mod::callee', 'function', 20, 0)",
            [file_id],
        )
        .unwrap();
    let tgt_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
         VALUES (?1, ?2, 'calls', 5, 0.50)",
            rusqlite::params![src_id, tgt_id],
        )
        .unwrap();

    let result = get_diagnostics(&db, "src/a.rs", LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(result.low_confidence_count, 1);
    assert_eq!(
        result.diagnostics[0].kind,
        DiagnosticKind::LowConfidenceEdge
    );
    assert_eq!(result.diagnostics[0].confidence, Some(0.50));
}

/// Seed two edges at different confidences + strategies, verify the
/// project-wide roll-up groups them correctly.
fn seed_edge(
    db: &Database,
    file_id: i64,
    src_name: &str,
    tgt_name: &str,
    confidence: f64,
    strategy: Option<&str>,
) {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, ?2, ?3, 'function', 1, 0)",
        rusqlite::params![file_id, src_name, format!("mod::{src_name}")],
    )
    .unwrap();
    let src_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, ?2, ?3, 'function', 5, 0)",
        rusqlite::params![file_id, tgt_name, format!("mod::{tgt_name}")],
    )
    .unwrap();
    let tgt_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence, strategy)
         VALUES (?1, ?2, 'calls', 1, ?3, ?4)",
        rusqlite::params![src_id, tgt_id, confidence, strategy],
    )
    .unwrap();
}

#[test]
fn low_confidence_edges_buckets_by_strategy_and_kind() {
    let db = Database::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin)
             VALUES ('src/a.rs', 'h', 'rust', 0, 'internal')",
            [],
        )
        .unwrap();
    let file_id = db.conn().last_insert_rowid();

    seed_edge(&db, file_id, "a1", "a2", 0.50, Some("heuristic_name_kind"));
    seed_edge(&db, file_id, "b1", "b2", 0.35, Some("heuristic_name_kind"));
    seed_edge(&db, file_id, "c1", "c2", 0.95, Some("ts_chain_resolution"));
    seed_edge(
        &db,
        file_id,
        "d1",
        "d2",
        1.00,
        Some("csharp_same_namespace"),
    );

    let report = low_confidence_edges(&db, LOW_CONFIDENCE_THRESHOLD).unwrap();
    // 0.50, 0.35, 0.95 all < 0.80? No — 0.95 > 0.80, so only 0.50/0.35
    // actually fall under the default threshold.
    assert_eq!(report.total, 2);
    assert_eq!(report.buckets.len(), 1);
    let b = &report.buckets[0];
    assert_eq!(b.strategy.as_deref(), Some("heuristic_name_kind"));
    assert_eq!(b.kind, "calls");
    assert_eq!(b.count, 2);
    assert!((b.min_confidence - 0.35).abs() < 1e-9);
    assert!((b.max_confidence - 0.50).abs() < 1e-9);
}

#[test]
fn low_confidence_edges_excludes_external_origin() {
    let db = Database::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin)
             VALUES ('ext/pkg/x.ts', 'h', 'typescript', 0, 'external')",
            [],
        )
        .unwrap();
    let file_id = db.conn().last_insert_rowid();
    seed_edge(&db, file_id, "x1", "x2", 0.50, Some("heuristic_name_kind"));

    let report = low_confidence_edges(&db, LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(report.total, 0);
}

// ---------------------------------------------------------------------------
// workspace_diagnostics — drained-ref exclusion
// ---------------------------------------------------------------------------

#[test]
fn workspace_diagnostics_excludes_drained_refs() {
    let db = Database::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin)
             VALUES ('script.sh', 'h', 'bash', 0, 'internal')",
            [],
        )
        .unwrap();
    let file_id = db.conn().last_insert_rowid();
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, origin)
             VALUES (?1, 'caller', 'mod::caller', 'function', 1, 0, 'internal')",
            [file_id],
        )
        .unwrap();
    let sym_id = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line)
             VALUES (?1, 'real_miss', 'calls', 2)",
            [sym_id],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line, drained)
             VALUES (?1, 'echo', 'calls', 3, 1)",
            [sym_id],
        )
        .unwrap();

    let report = workspace_diagnostics(&db, 10, LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(report.total_unresolved, 1);
    assert_eq!(report.top_files_by_unresolved[0].unresolved_count, 1);
}
