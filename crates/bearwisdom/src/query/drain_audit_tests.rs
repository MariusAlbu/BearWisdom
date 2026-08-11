use super::*;

fn setup_db() -> Database {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed, origin) \
         VALUES ('src/main.pas', 'h1', 'pascal', 0, 'internal')",
        [],
    )
    .unwrap();
    let file_id = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'caller', 'caller', 'function', 1, 0)",
        [file_id],
    )
    .unwrap();
    let caller_id = conn.last_insert_rowid();

    // Drained refs: one masking a real function, one masking only a
    // variable (kind-incompatible for Calls), one masking nothing.
    for target in ["FreeAndNil", "SizeOf", "Inc"] {
        conn.execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, drained) \
             VALUES (?1, ?2, 'calls', 1)",
            rusqlite::params![caller_id, target],
        )
        .unwrap();
    }

    // A real internal function shadowing FreeAndNil (case-folded, as a
    // case-insensitive language declares it).
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'freeandnil', 'shims.freeandnil', 'function', 30, 0)",
        [file_id],
    )
    .unwrap();
    // A variable named SizeOf — a Calls drain must not flag it.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
         VALUES (?1, 'SizeOf', 'cfg.SizeOf', 'variable', 40, 0)",
        [file_id],
    )
    .unwrap();

    db
}

#[test]
fn flags_drained_name_with_compatible_declaration() {
    let db = setup_db();
    let findings = drain_audit(&db).unwrap();
    assert_eq!(findings.len(), 1, "only FreeAndNil should flag: {findings:?}");
    let f = &findings[0];
    assert_eq!(f.target_name, "FreeAndNil");
    assert_eq!(f.drained_count, 1);
    assert_eq!(f.total_matches, 1);
    assert_eq!(f.matches[0].origin, "internal");
    assert!(!f.matches[0].case_exact, "declared lowercase, drained mixed-case");
}

#[test]
fn kind_incompatible_declarations_do_not_flag() {
    let db = setup_db();
    let findings = drain_audit(&db).unwrap();
    assert!(
        findings.iter().all(|f| f.target_name != "SizeOf"),
        "a variable must not satisfy a Calls-kind drain: {findings:?}"
    );
}

#[test]
fn undeclared_drained_names_stay_silent() {
    let db = setup_db();
    let findings = drain_audit(&db).unwrap();
    assert!(findings.iter().all(|f| f.target_name != "Inc"));
}

#[test]
fn empty_db_yields_no_findings() {
    let db = Database::open_in_memory().unwrap();
    assert!(drain_audit(&db).unwrap().is_empty());
}
