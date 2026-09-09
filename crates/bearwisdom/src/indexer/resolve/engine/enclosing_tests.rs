use super::super::{compilation::Compilation, contract::SymbolLookup};
use super::*;
use crate::type_checker::core::types::TypeArena;
use std::sync::Arc;

#[test]
fn enclosing_id_restore_uses_nearest_unique_live_parent_and_preserves_fresh_facts() {
    let symbols: FxHashMap<_, _> = [
        (1, "class"),
        (2, "class"),
        (3, "method"),
        (4, "function"),
        (5, "method"),
        (6, "function"),
        (7, "function"),
        (8, "function"),
        (9, "function"),
        (10, "method"),
        (11, "class"),
        (12, "method"),
        (13, "method"),
        (14, "method"),
    ]
    .into_iter()
    .map(|(id, kind)| {
        (
            id,
            super::super::testkit::sym(id, "same", "same", kind, "same.ts"),
        )
    })
    .collect();
    let members = FxHashMap::from_iter([
        (1, vec![3, 10, 11, 13, 14]),
        (2, vec![5, 10]),
        (3, vec![4]),
        (11, vec![12]),
        (6, vec![6]),
        (7, vec![8]),
        (8, vec![7]),
        (999, vec![9]),
    ]);
    let mut enclosing = FxHashMap::from_iter([(13, 2)]);
    restore_enclosing_types(
        &symbols,
        &members,
        &FxHashSet::from_iter([13, 14]),
        &mut enclosing,
    );
    assert_eq!(
        enclosing,
        FxHashMap::from_iter([(3, 1), (4, 1), (5, 2), (11, 1), (12, 11), (13, 2)])
    );
    // A deleted parent cannot preserve an earlier recovered result.
    let mut removed = symbols.clone();
    removed.remove(&1);
    restore_enclosing_types(
        &removed,
        &members,
        &FxHashSet::from_iter([13, 14]),
        &mut enclosing,
    );
    assert!(!enclosing.contains_key(&3));
    assert!(!enclosing.contains_key(&4));
    assert_eq!(enclosing.get(&13), Some(&2));
}

#[test]
fn db_loaded_declarations_retain_exact_enclosing_type_ids() {
    let dir = tempfile::tempdir().unwrap();
    let arena = Arc::new(TypeArena::new());
    let source = "export class Service<T> { run() { const callback = () => this.run(); } }";
    let mut parsed = Vec::new();
    for name in ["a.ts", "b.ts"] {
        let path = dir.path().join(name);
        std::fs::write(&path, source).unwrap();
        parsed.push(
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: name.into(),
                    absolute_path: path,
                    language: "typescript",
                },
                crate::languages::default_registry(),
                &arena,
            )
            .unwrap(),
        );
    }
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let fresh = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    fresh.persist_type_info(db.conn()).unwrap();
    let snapshot: String = db
        .conn()
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for changed in [0, 1] {
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&snapshot);
        let mut cold = Compilation::build(&parsed[..changed], &ids, restored);
        cold.ingest_from_db(db.conn());
        for file in &parsed {
            let owner = file
                .symbols
                .iter()
                .position(|s| s.kind == crate::types::SymbolKind::Class)
                .unwrap();
            let expected = ids.row_id(&file.path, owner).unwrap();
            for (slot, symbol) in file
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, s)| s.parent_index.is_some())
            {
                let id = ids.row_id(&file.path, slot).unwrap();
                assert_eq!(
                    fresh.enclosing_type_id_of(id),
                    Some(expected),
                    "fresh {symbol:?}"
                );
                assert_eq!(
                    cold.enclosing_type_id_of(id),
                    Some(expected),
                    "cold {changed} {} slot {slot}",
                    file.path
                );
            }
        }
    }
}
