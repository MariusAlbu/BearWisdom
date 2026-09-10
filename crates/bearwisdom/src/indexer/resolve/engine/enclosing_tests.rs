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

#[test]
fn scala_this_chain_selects_its_own_same_qname_member_fresh_and_cold() {
    use crate::indexer::resolve::engine::{
        chain::bind_member_access,
        contract::{FileContext, RefContext},
        file_lookup::FileLookup,
    };
    use crate::types::{EdgeKind, SegmentKind, SymbolKind};

    let dir = tempfile::tempdir().unwrap();
    let arena = Arc::new(TypeArena::new());
    let source = "class Same { def modify() = (); def call() = this.modify() }";
    let mut parsed = Vec::new();
    for name in ["a.scala", "b.scala"] {
        let path = dir.path().join(name);
        std::fs::write(&path, source).unwrap();
        parsed.push(
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: name.into(),
                    absolute_path: path,
                    language: "scala",
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
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&snapshot);
    let mut cold = Compilation::build(
        &[],
        &crate::indexer::write::SymbolIds::default(),
        Arc::clone(&restored),
    );
    cold.ingest_from_db(db.conn());

    for file in &parsed {
        let owner_slot = file
            .symbols
            .iter()
            .position(|s| s.kind == SymbolKind::Class && s.name == "Same")
            .unwrap();
        let modify_slot = file
            .symbols
            .iter()
            .position(|s| s.kind == SymbolKind::Method && s.name == "modify")
            .unwrap();
        let call = file
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "modify")
            .unwrap();
        let segments = &call.chain.as_ref().unwrap().segments;
        assert!(matches!(segments[0].kind, SegmentKind::SelfRef));
        assert!(segments[1].is_call);

        let owner_id = ids.row_id(&file.path, owner_slot).unwrap();
        let expected = ids.row_id(&file.path, modify_slot).unwrap();
        let source_id = ids.row_id(&file.path, call.source_symbol_index).unwrap();
        for (mode, tree) in [("fresh", &fresh), ("cold", &cold)] {
            assert_eq!(
                tree.enclosing_type_id_of(source_id),
                Some(owner_id),
                "{mode} owner for {}",
                file.path
            );
            let context = RefContext {
                extracted_ref: call,
                source_symbol: &file.symbols[call.source_symbol_index],
                scope_chain: vec![],
                file_package_id: file.package_id,
                source_symbol_id: Some(source_id),
            };
            let file_context = FileContext {
                file_path: file.path.clone(),
                language: "scala".into(),
                imports: vec![],
                file_namespace: None,
            };
            let lookup = FileLookup::for_file(tree, file, &ids);
            let result = bind_member_access(
                &context,
                &file_context,
                &lookup,
                &crate::languages::scala::profile::SCALA_PROFILE,
            )
            .unwrap_or_else(|cause| panic!("{mode} {}: {cause:?}", file.path));
            assert_eq!(
                result.target_symbol_id, expected,
                "{mode} {} must not bind the other Same.modify",
                file.path
            );
        }
    }
}
