use super::*;
use crate::db::Database;

/// A same-file call — no import machinery involved — resolves through the
/// real pipeline, giving a deterministic `ref_resolutions` row to export and
/// round-trip through JSONL.
#[test]
fn export_and_jsonl_round_trip_on_a_small_fixture_project() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("lib.ts"),
        "export function helper() {\n  return 1;\n}\n\nexport function caller() {\n  return helper();\n}\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    crate::full_index(&mut db, dir.path(), None, None, None).unwrap();

    let exported = export_ref_snapshot(&db).unwrap();
    assert!(!exported.is_empty(), "expected at least one logged ref");
    let resolved_call = exported
        .iter()
        .find(|e| e.target == "helper" && e.kind == "calls")
        .expect("caller() -> helper() must be logged");
    assert_eq!(resolved_call.outcome, "resolved");
    assert!(resolved_call.target_qname.is_some());

    // Deterministic: re-exporting the same DB state yields byte-identical rows.
    let exported_again = export_ref_snapshot(&db).unwrap();
    assert_eq!(exported, exported_again, "export must be deterministic");

    // JSONL round trip preserves every field.
    let mut buf: Vec<u8> = Vec::new();
    write_snapshot_jsonl(&exported, &mut buf).unwrap();
    let snapshot_path = dir.path().join("snapshot.jsonl");
    std::fs::write(&snapshot_path, &buf).unwrap();
    let read_back = read_snapshot_jsonl(&snapshot_path).unwrap();
    assert_eq!(read_back, exported, "JSONL round trip must be lossless");
}

#[test]
fn read_snapshot_jsonl_skips_blank_lines() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("snapshot.jsonl");
    std::fs::write(
        &path,
        "{\"file\":\"a.ts\",\"line\":1,\"col\":0,\"target\":\"x\",\"kind\":\"calls\",\"outcome\":\"unresolved\"}\n\n",
    )
    .unwrap();
    let rows = read_snapshot_jsonl(&path).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target, "x");
}
