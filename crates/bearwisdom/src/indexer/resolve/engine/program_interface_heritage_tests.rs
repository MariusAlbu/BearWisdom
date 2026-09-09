use super::super::tests::{context, owner, parse};
use super::*;
use crate::indexer::symbol_ids::SymbolIds;
use std::{collections::HashSet, sync::Arc};

fn selected(tree: &Compilation, child: i64, parent: i64, member: i64) {
    use crate::indexer::resolve::engine::member_selection::{select, Selection};
    let lookup = tree.program_lookup("child.ts").unwrap();
    assert_eq!(lookup.parent_class_ids(child), vec![parent]);
    let name = lookup.member_index().unwrap().name("read").unwrap();
    assert_eq!(
        select(&lookup, child, name, &|_| true),
        Selection::Unique(member)
    );
    let args = lookup.parent_class_arg_ids_of(child, parent);
    assert_eq!(args.len(), 1);
    let arena = tree.type_arena().unwrap();
    assert_eq!(
        arena.get(args[0]),
        Type::Generic {
            param: lookup.canonical_type_info(child).unwrap().generic_param_ids[0]
        }
    );
}

#[test]
fn interface_heritage_admission_matches_compiler_diagnostic_evidence_fresh_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/interface_compatibility_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let arena = Arc::new(TypeArena::new());
        let files = parse(&arena, &[("main.ts", case["source"].as_str().unwrap())]);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let row = owner(&ids, &files[0], "Catalog");
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&[&files[0]])),
            &HashSet::new(),
        );
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            assert_eq!(
                lookup.symbol_by_id(row).is_some(),
                case["admitted"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        };
        check(&tree);
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold);
    }
}

#[test]
fn interface_heritage_rebinds_unchanged_consumers_after_barrel_retarget_and_deletion() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("left.ts", "export interface Root<T> { read(): T; }"),
        ("right.ts", "export interface Root<T> { read(): T; }"),
        ("barrel.ts", "export type { Root } from './left';"),
        ("child.ts", "import type { Root } from './barrel'; export interface Child<T> extends Root<T> {}")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let child = owner(&ids, &files[3], "Child");
    let left = (
        owner(&ids, &files[0], "Root"),
        owner(&ids, &files[0], "read"),
    );
    let right = (
        owner(&ids, &files[1], "Root"),
        owner(&ids, &files[1], "read"),
    );
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    selected(&tree, child, left.0, left.1);
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("barrel.ts", "export type { Root } from './right';")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let config = context(&[&files[0], &files[1], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    selected(&edited, child, right.0, right.1);
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    selected(&cold, child, right.0, right.1);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let remaining = context(&[&files[0], &changed[0], &files[3]]);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        restored,
        Some(&remaining),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    let lookup = deleted.program_lookup("child.ts").unwrap();
    assert!(
        lookup.symbol_by_id(left.0).is_some(),
        "unrelated namesake provider remains supplied"
    );
    assert!(
        lookup.symbol_by_id(child).is_none(),
        "missing imported base must not borrow that provider"
    );
    assert!(lookup.parent_class_ids(child).is_empty());
}

#[test]
fn portable_interface_inputs_keep_program_local_generic_and_inherited_member_ids() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source =
        "export interface Root<T> { read(): T; } export interface Child<T> extends Root<T> {}";
    let mut files = parse(&original, &[("child.ts", source)]);
    reduce_to_contract(&mut files[0]);
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "child.ts",
        &files[0].content_hash,
        files[0].size,
        None,
    );
    file.content = Some(source.into());
    reduce_to_contract(&mut file);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let child = owner(&ids, &file, "Child");
    let parent = owner(&ids, &file, "Root");
    let member = owner(&ids, &file, "read");
    for symbol in &mut file.symbols {
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    selected(&tree, child, parent, member);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    selected(&cold, child, parent, member);
}

#[test]
fn interface_heritage_proofs_do_not_leak_across_overlapping_programs() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("base.d.ts", "interface Root<T> { value: T; read(): T; }"),
            (
                "good.ts",
                "export {}; interface Child extends Root<string> { value: string; }",
            ),
            (
                "bad.ts",
                "export {}; interface Child extends Root<number> { value: string; }",
            ),
        ],
    );
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let good = owner(&ids, &files[1], "Child");
    let bad = owner(&ids, &files[2], "Child");
    let base = owner(&ids, &files[0], "Root");
    let mut config = context(&[&files[0], &files[1]]);
    let mut other = context(&[&files[0], &files[2]]).programs.unwrap().remove(0);
    other.key = "other".into();
    other.fingerprint = "other".into();
    config.programs.as_mut().unwrap().push(other);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let a = tree.program_lookup("good.ts").unwrap();
        let b = tree.program_lookup("bad.ts").unwrap();
        assert!(a.symbol_by_id(good).is_some());
        assert!(a.symbol_by_id(bad).is_none());
        assert!(b.symbol_by_id(bad).is_none());
        assert!(b.symbol_by_id(good).is_none());
        assert!(a.symbol_by_id(base).is_some());
        assert!(
            b.symbol_by_id(base).is_some(),
            "bad child does not invalidate its legal base"
        );
        assert!(
            tree.program_lookup("base.d.ts")
                .unwrap()
                .symbol_by_id(base)
                .is_none(),
            "shared source needs explicit program selection"
        );
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}
