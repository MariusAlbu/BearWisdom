use super::*;

#[test]
fn base_receivers_retarget_unchanged_callers_and_reject_deleted_or_stale_providers() {
    use super::super::super::{
        base_receiver, chain, file_lookup::FileLookup, head_decl::head_decl_id, testkit,
    };
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("a.ts", "export class Doc { touch() {} } export class Parent<T> { next(): T { throw 0; } }"),
        ("b.ts", "export class Doc { touch() {} } export class Parent<T> { next(): T { throw 0; } }"),
        ("barrel.ts", "export { Parent, Doc } from './a';"),
        ("use.ts", "import { Parent as Base, Doc } from './barrel'; class Child extends Base<Doc> { run() { super.next().touch(); } }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let child = row(3, "Child");
    let check = |tree: &Compilation, expected: Option<usize>| {
        let caller = &parsed[3];
        let lookup = FileLookup::for_file(tree, caller, &ids);
        let reference = caller
            .refs
            .iter()
            .find(|r| {
                r.kind == EdgeKind::Calls
                    && r.chain
                        .as_ref()
                        .is_some_and(|c| c.segments.last().is_some_and(|s| s.name == "touch"))
            })
            .unwrap();
        let mut context = testkit::ref_ctx(
            reference,
            &caller.symbols[reference.source_symbol_index],
            vec![],
        );
        context.source_symbol_id = ids.row_id(&caller.path, reference.source_symbol_index);
        let root = base_receiver::root(&context, &lookup, &tree.arena);
        let result = chain::bind_member_access(
            &context,
            &testkit::file_ctx(vec![], None),
            &lookup,
            &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
        );
        if let Some(file) = expected {
            assert_eq!(
                head_decl_id(&tree.arena, root.unwrap().ty),
                Some(row(file, "Parent"))
            );
            assert_eq!(result.unwrap().target_symbol_id, row(file, "touch"));
            assert_eq!(tree.parent_class_ids(child), [row(file, "Parent")]);
        } else {
            assert!(root.is_none());
            assert!(
                result.is_err(),
                "missing provider cannot borrow the other namesake"
            );
        }
    };
    let mut tree = Compilation::build(&parsed[2..], &ids, Arc::clone(&arena));
    check(&tree, None);
    tree.ingest(&parsed[..2], &ids, &HashSet::new());
    check(&tree, Some(0));
    let inputs = serde_json::to_string(&tree.modules.inputs["use.ts"].bases).unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse_modules(
        &arena,
        &[("barrel.ts", "export { Parent, Doc } from './b';")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut incremental = Compilation::build(&changed, &changed_ids, Arc::clone(&arena));
    incremental.ingest_from_db(db.conn());
    check(&incremental, Some(1));
    assert_eq!(
        serde_json::to_string(&incremental.modules.inputs["use.ts"].bases).unwrap(),
        inputs
    );
    incremental.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(1));
    db.conn()
        .execute("DELETE FROM files WHERE path='b.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
    assert_eq!(
        restored.get(
            deleted
                .canonical_type_info(child)
                .unwrap()
                .base_type_id
                .unwrap()
        ),
        Type::Unknown
    );
    db.conn()
        .execute("UPDATE files SET hash='changed' WHERE path='use.ts'", [])
        .unwrap();
    let mut stale = Compilation::build(&[], &SymbolIds::default(), restored);
    stale.ingest_from_db(db.conn());
    check(&stale, None);
    assert!(
        stale
            .canonical_type_info(child)
            .unwrap()
            .base_type_id
            .is_none(),
        "rejected source input cannot retain a cached base receiver"
    );
}

fn parse_modules(arena: &TypeArena, sources: &[(&str, &str)]) -> Vec<ParsedFile> {
    let dir = tempfile::tempdir().unwrap();
    let registry = crate::languages::default_registry();
    sources
        .iter()
        .map(|(path, source)| {
            let absolute_path = dir.path().join(path);
            std::fs::write(&absolute_path, source).unwrap();
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: (*path).into(),
                    absolute_path,
                    language: "typescript",
                },
                &registry,
                arena,
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn base_receivers_keep_filtered_and_portable_provider_generic_identities() {
    use super::super::super::{chain, file_lookup::FileLookup, testkit};
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let source_arena = TypeArena::new();
    let mut source = parse_modules(&source_arena, &[("provider.ts", "export function discard() { class Parent { wrong() {} } } export class Doc { touch() {} } export class Ancestor<T> { next(): T { throw 0; } } export class Parent<U> extends Ancestor<U> {}")]);
    let provider = &mut source[0];
    let before = provider.symbols.len();
    reduce_to_contract(provider);
    assert!(provider.symbols.len() < before);
    let cached = serde_json::to_string(&CachedParse::from_parsed(provider, &source_arena)).unwrap();
    let arena = Arc::new(TypeArena::new());
    arena.intern_generic(GenericParamData {
        name: "noise".into(),
        owner_symbol_index: 999,
        bound: None,
        kind: Default::default(),
    });
    let cached: CachedParse = serde_json::from_str(&cached).unwrap();
    let mut restored = cached.into_parsed(
        &arena,
        "provider.ts",
        &provider.content_hash,
        provider.size,
        None,
    );
    restored.content = provider.content.clone();
    reduce_to_contract(&mut restored);
    assert_eq!(
        restored
            .symbols
            .iter()
            .filter(|s| s.name == "Parent")
            .count(),
        1,
        "private namesake was filtered before source recapture"
    );
    let mut parsed = vec![restored];
    parsed.extend(parse_modules(&arena, &[("use.ts", "import { Parent, Doc } from './provider'; class Child extends Parent<Doc> { run() { super.next().touch(); } }")]));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    tree.persist_type_info(db.conn()).unwrap();
    let snapshot = Arc::new(TypeArena::new());
    snapshot.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), snapshot);
    cold.ingest_from_db(db.conn());
    let expected = ids
        .row_id(
            "provider.ts",
            parsed[0]
                .symbols
                .iter()
                .position(|s| s.name == "touch")
                .unwrap(),
        )
        .unwrap();
    for tree in [&tree, &cold] {
        let caller = &parsed[1];
        let lookup = FileLookup::for_file(tree, caller, &ids);
        let reference = caller
            .refs
            .iter()
            .find(|r| {
                r.kind == EdgeKind::Calls
                    && r.chain
                        .as_ref()
                        .is_some_and(|c| c.segments.last().is_some_and(|s| s.name == "touch"))
            })
            .unwrap();
        let mut context = testkit::ref_ctx(
            reference,
            &caller.symbols[reference.source_symbol_index],
            vec![],
        );
        context.source_symbol_id = ids.row_id(&caller.path, reference.source_symbol_index);
        let result = chain::bind_member_access(
            &context,
            &testkit::file_ctx(vec![], None),
            &lookup,
            &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
        )
        .unwrap();
        assert_eq!(result.target_symbol_id, expected);
    }
}

#[test]
fn scoped_merge_groups_survive_reload_and_split_after_a_source_edit() {
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("api.ts", "export interface Model { save(): void; } export interface Model { reload(): void; }"),
        ("use.ts", "import { Model } from './api'; export function create<T>(): Model { throw 0; }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let rows: Vec<_> = parsed[0]
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SymbolKind::Interface)
        .map(|(slot, _)| ids.row_id("api.ts", slot).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    let create = tree.by_name("create").first().unwrap().id;
    let root = tree.canonical_decl_id(rows[0]);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    for tree in [&tree, &cold] {
        assert_eq!(tree.canonical_decl_id(rows[1]), root);
        let members: std::collections::BTreeSet<_> = tree
            .members_of_id(root)
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(
            members,
            ["save".into(), "reload".into()].into_iter().collect()
        );
        assert_eq!(
            super::super::super::head_decl::head_decl_id(
                &restored,
                tree.return_type_id_of(create).unwrap()
            ),
            Some(root)
        );
    }
    let changed = parse_modules(&restored, &[("api.ts",
        "export interface Model { save(): void; } function nested() { interface Model { reload(): void; } }")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&restored),
    )
    .unwrap();
    let mut incremental = Compilation::build(&changed, &changed_ids, Arc::clone(&restored));
    incremental.ingest_from_db(db.conn());
    let roots: Vec<_> = changed[0]
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SymbolKind::Interface)
        .map(|(slot, _)| changed_ids.row_id("api.ts", slot).unwrap())
        .collect();
    assert_eq!(roots.len(), 2);
    assert_ne!(
        incremental.canonical_decl_id(roots[0]),
        incremental.canonical_decl_id(roots[1])
    );
    assert_eq!(
        incremental
            .members_of_id(roots[0])
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["save"]
    );
    assert_eq!(
        incremental
            .members_of_id(roots[1])
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["reload"]
    );
    assert_eq!(
        super::super::super::head_decl::head_decl_id(
            &restored,
            incremental.return_type_id_of(create).unwrap()
        ),
        Some(roots[0])
    );
    incremental.persist_type_info(db.conn()).unwrap();
    let mut split_cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    split_cold.ingest_from_db(db.conn());
    assert_ne!(
        split_cold.canonical_decl_id(roots[0]),
        split_cold.canonical_decl_id(roots[1])
    );
    assert_eq!(
        split_cold
            .members_of_id(roots[0])
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["save"]
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='api.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    assert_eq!(
        restored.get(deleted.return_type_id_of(create).unwrap()),
        Type::Unknown
    );
    assert!(deleted.members_of_id(roots[0]).is_empty());
}

#[test]
fn scoped_merge_rejects_duplicate_classes_aliases_and_incompatible_arity() {
    for source in [
        "class Model {} class Model {}",
        "type Model = string; type Model = number;",
        "interface Model<T> {} interface Model<T, U> {}",
    ] {
        let arena = Arc::new(TypeArena::new());
        let parsed = parse_modules(&arena, &[("api.ts", source)]);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &parsed,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
        let graph = parsed[0].flow.lexical.as_ref().unwrap();
        assert!(graph.mergeable_types.is_empty(), "{source}");
        let types: Vec<_> = graph
            .type_symbol_slots
            .values()
            .flatten()
            .map(|&slot| ids.row_id("api.ts", slot).unwrap())
            .collect();
        assert_eq!(types.len(), 2, "{source}");
        assert_ne!(
            tree.canonical_decl_id(types[0]),
            tree.canonical_decl_id(types[1]),
            "{source}"
        );
    }
}

#[test]
fn namespace_overload_selection_and_type_only_value_uses_abstain_after_reload() {
    use super::super::super::{
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("api.d.ts", "export declare function create(value: string): void; export declare function create(value: number): void;"),
        ("use.ts", "import * as api from './api'; import type * as types from './api'; function f() { api.create(1); types.create(1); }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    tree.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    cold.ingest_from_db(db.conn());
    for tree in [&tree, &cold] {
        let lookup = FileLookup::for_file(tree, &parsed[1], &ids);
        let source = testkit::source_symbol("f");
        let calls: Vec<_> = parsed[1]
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .collect();
        let expected: std::collections::BTreeSet<_> = ["api.create(1)", "types.create(1)"]
            .iter()
            .map(|text| parsed[1].content.as_ref().unwrap().find(text).unwrap() as u32)
            .collect();
        assert_eq!(
            calls
                .iter()
                .map(|r| r.byte_offset)
                .collect::<std::collections::BTreeSet<_>>(),
            expected,
            "extracted duplicates must still refer to the two source calls"
        );
        for reference in calls {
            lookup.set_cursor(reference.byte_offset);
            assert!(matches!(
                SemanticModel::production().get_symbol_info(
                    &testkit::ref_ctx(reference, &source, vec![]),
                    &testkit::file_ctx(vec![], None),
                    &lookup,
                    &crate::languages::typescript::profile::TYPESCRIPT_PROFILE
                ),
                SolveOutcome::Unresolved(_)
            ));
        }
        let graph = parsed[1].flow.lexical.as_ref().unwrap();
        let real = graph
            .module
            .imports
            .iter()
            .find(|(_, i)| !i.type_only && i.selectors.len() == 1)
            .unwrap()
            .0;
        let erased = graph
            .module
            .imports
            .iter()
            .find(|(_, i)| i.type_only && i.selectors.len() == 1)
            .unwrap()
            .0;
        assert_eq!(tree.bound_import_overloads("use.ts", *real).len(), 2);
        assert!(tree.bound_import_overloads("use.ts", *erased).is_empty());
    }
}

#[test]
fn namespace_paths_rebind_qualified_returns_after_barrel_edit_reload_and_deletion() {
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("a.ts", "export class Model { save(): void {} }"),
        ("b.ts", "export class Model { save(): void {} }"),
        ("barrel.ts", "export * as API from './a';"),
        ("api.ts", "import { API } from './barrel'; export function create<T>(): API.Model { throw 0; }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let create = tree.by_name("create").first().unwrap().id;
    let a = tree
        .by_name("Model")
        .iter()
        .find(|s| s.file_path.as_ref() == "a.ts")
        .unwrap()
        .id;
    let b = tree
        .by_name("Model")
        .iter()
        .find(|s| s.file_path.as_ref() == "b.ts")
        .unwrap()
        .id;
    assert_eq!(tree.return_type_id_of(create), Some(arena.decl("Model", a)));
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse_modules(&arena, &[("barrel.ts", "export * as API from './b';")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut incremental = Compilation::build(&changed, &changed_ids, Arc::clone(&arena));
    incremental.ingest_from_db(db.conn());
    assert_eq!(
        incremental.return_type_id_of(create),
        Some(arena.decl("Model", b))
    );
    assert_eq!(
        incremental
            .generic_return_of(create)
            .unwrap()
            .instantiate(&arena, &[arena.intern(Type::Unknown)]),
        Some(arena.decl("Model", b))
    );
    incremental.persist_type_info(db.conn()).unwrap();
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
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    assert_eq!(
        cold.return_type_id_of(create),
        Some(restored.decl("Model", b))
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='b.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    assert_eq!(
        restored.get(deleted.return_type_id_of(create).unwrap()),
        Type::Unknown
    );
}

#[test]
fn declaration_file_callable_and_value_exports_join_exact_symbol_slots() {
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("api.d.ts", "export interface Result { refetch(): void; } export declare function create(): Result; export declare const api: Result;"),
        ("use.ts", "import { create, api } from './api'; function f() { create().refetch(); api.refetch(); }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let graph = parsed[1].flow.lexical.as_ref().unwrap();
    for (&binding, import) in &graph.module.imports {
        if !import.selectors.is_empty() {
            continue;
        } // This test checks declared imports, not detached selections.
        let target = tree.bound_import("use.ts", binding, false);
        assert!(
            target.is_some(),
            "{:?}: exports {:?}, slots {:?}, symbols {:?}",
            import.source,
            parsed[0].flow.lexical.as_ref().unwrap().module.exports,
            parsed[0].flow.lexical.as_ref().unwrap().symbol_slots,
            parsed[0]
                .symbols
                .iter()
                .map(|s| (&s.name, s.kind, s.start_line, s.start_col))
                .collect::<Vec<_>>()
        );
        let symbol = tree.symbol_by_id(target.unwrap()).unwrap();
        let ty = if symbol.kind == "function" {
            tree.return_type_id_of(symbol.id)
        } else {
            tree.field_type_id_of(symbol.id)
        };
        assert!(
            ty.and_then(|ty| super::super::super::head_decl::head_decl_id(&arena, ty))
                .is_some(),
            "{:?}: {ty:?}",
            import.source
        );
    }
}

#[test]
fn imported_overload_groups_keep_all_rows_without_selecting_a_navigation_target() {
    use super::super::super::{
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("api.d.ts", "export interface Model { save(): void; } export declare function create(value: string): Model; export declare function create(value: number): Model;"),
        ("use.ts", "import { create } from './api'; function f() { create(1); }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let binding = *parsed[1]
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .module
        .imports
        .keys()
        .next()
        .unwrap();
    let mut expected: Vec<_> = parsed[0]
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SymbolKind::Function)
        .map(|(slot, _)| ids.row_id(&parsed[0].path, slot).unwrap())
        .collect();
    expected.sort_unstable();
    assert_eq!(expected.len(), 2);
    assert_eq!(tree.bound_import_overloads("use.ts", binding), expected);
    assert_eq!(
        tree.bound_import("use.ts", binding, false),
        None,
        "a group is not a selected overload"
    );
    let lookup = FileLookup::for_file(&tree, &parsed[1], &ids);
    let reference = parsed[1]
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls)
        .unwrap();
    lookup.set_cursor(reference.byte_offset);
    assert_eq!(
        lookup
            .local_reference(reference.byte_offset)
            .unwrap()
            .declaration,
        None
    );
    let source = testkit::source_symbol("f");
    let context = testkit::ref_ctx(reference, &source, vec![]);
    assert!(matches!(
        SemanticModel::production().get_symbol_info(
            &context,
            &testkit::file_ctx(vec![], None),
            &lookup,
            &crate::languages::typescript::profile::TYPESCRIPT_PROFILE
        ),
        SolveOutcome::Unresolved(None)
    ));
    tree.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), arena);
    cold.ingest_from_db(db.conn());
    assert_eq!(cold.bound_import_overloads("use.ts", binding), expected);
    assert_eq!(cold.bound_import("use.ts", binding, false), None);
}

#[test]
fn known_package_entry_survives_cold_reload_and_deletion() {
    let arena = Arc::new(TypeArena::new());
    let mut parsed = parse_modules(
        &arena,
        &[
            ("package.d.ts", "export interface Model { save(): void; }"),
            (
                "api.ts",
                "import { Model } from 'demo'; export function create(): Model { throw 0; }",
            ),
        ],
    );
    parsed[0].path = "ext:ts:demo/index.d.ts".into();
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let binding = *parsed[1]
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .module
        .imports
        .keys()
        .next()
        .unwrap();
    let target = tree
        .bound_import("api.ts", binding, true)
        .expect("package entry resolves");
    tree.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    cold.ingest_from_db(db.conn());
    assert_eq!(cold.bound_import("api.ts", binding, true), Some(target));
    db.conn()
        .execute("DELETE FROM files WHERE path='ext:ts:demo/index.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), arena);
    deleted.ingest_from_db(db.conn());
    assert_eq!(deleted.bound_import("api.ts", binding, true), None);
}

#[test]
fn imported_type_annotations_keep_exact_rows_before_member_resolution() {
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("left.ts", "class Model { save(): void {} } export { Model };"),
        ("right.ts", "class Model { save(): void {} } export { Model };"),
        ("use.ts", "import { Model as Left } from './left'; import { Model as Right } from './right'; function f(a: Left, b: Right) { a.save(); b.save(); }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let graph = parsed[2].flow.lexical.as_ref().unwrap();
    for (alias, file) in [("Left", "left.ts"), ("Right", "right.ts")] {
        let binding = graph
            .type_binding_at(100, graph.name_id(alias).unwrap())
            .unwrap();
        let target = tree.bound_import("use.ts", binding, true).unwrap();
        assert_eq!(tree.symbol_by_id(target).unwrap().file_path.as_ref(), file);
        let binder = super::super::super::lexical_type_ids::TypeBinder {
            graph,
            path: "use.ts",
            ids: &ids,
            lookup: &tree,
            source: Some(&tree),
            arena: &arena,
        };
        let ty = binder
            .materialize(&crate::indexer::lexical::type_syntax::TypeExpr::Declaration(binding));
        assert_eq!(
            super::super::super::head_decl::head_decl_id(&arena, ty),
            Some(target)
        );
        let members = tree.members_of_id(target);
        assert!(!members.is_empty());
        assert!(
            members
                .iter()
                .all(|member| member.file_path.as_ref() == file),
            "members belong to {file}"
        );
    }
}

#[test]
fn imported_signatures_rebind_after_late_batch_and_cold_reload_without_resurrecting_deleted_modules(
) {
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(&arena, &[
        ("model.ts", "export class Model { save(): void {} }"),
        ("api.ts", "import { Model as Item } from './model'; export function create(): Item { throw 0; } export function consume(callback: () => Item): Item { throw 0; }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut tree = Compilation::build(&parsed[1..], &ids, Arc::clone(&arena));
    let create = tree.by_name("create").iter().next().unwrap().id;
    let consume = tree.by_name("consume").iter().next().unwrap().id;
    assert_eq!(
        arena.get(tree.return_type_id_of(create).unwrap()),
        Type::Unknown
    );
    tree.ingest(&parsed[..1], &ids, &HashSet::new());
    let model = tree.by_name("Model").iter().next().unwrap().id;
    let expected = arena.decl("Model", model);
    assert_eq!(
        tree.return_type_id_of(create),
        Some(expected),
        "late dependency repairs source-bound returns"
    );
    assert_eq!(
        arena.get(
            tree.type_info_by_id[&consume]
                .parameter_type_ids
                .as_ref()
                .unwrap()[0]
        ),
        Type::Function {
            params: vec![],
            return_: expected
        }
    );
    tree.persist_type_info(db.conn()).unwrap();
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
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    let binding = *parsed[1]
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .module
        .imports
        .keys()
        .next()
        .unwrap();
    assert_eq!(
        cold.bound_import("api.ts", binding, true),
        Some(model),
        "cold snapshot keeps module edges"
    );
    assert_eq!(cold.return_type_id_of(create), Some(expected));
    db.conn()
        .execute("DELETE FROM files WHERE path='model.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    assert_eq!(deleted.bound_import("api.ts", binding, true), None);
    assert_eq!(
        restored.get(deleted.return_type_id_of(create).unwrap()),
        Type::Unknown,
        "a deleted dependency invalidates its imported signature head"
    );
}

#[test]
fn editing_a_barrel_retargets_unchanged_imported_signatures_and_survives_the_next_reload() {
    let arena = Arc::new(TypeArena::new());
    let parsed = parse_modules(
        &arena,
        &[
            ("a.ts", "export class Model { save(): void {} }"),
            ("b.ts", "export class Model { save(): void {} }"),
            ("barrel.ts", "export { Model } from './a';"),
            (
                "api.ts",
                "import { Model } from './barrel'; export function create<T>(): Model { throw 0; }",
            ),
        ],
    );
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let create = tree.by_name("create").iter().next().unwrap().id;
    let a = tree
        .by_name("Model")
        .iter()
        .find(|s| s.file_path.as_ref() == "a.ts")
        .unwrap()
        .id;
    let b = tree
        .by_name("Model")
        .iter()
        .find(|s| s.file_path.as_ref() == "b.ts")
        .unwrap()
        .id;
    assert_eq!(tree.return_type_id_of(create), Some(arena.decl("Model", a)));
    assert_eq!(
        tree.generic_return_of(create)
            .unwrap()
            .instantiate(&arena, &[arena.intern(Type::Unknown)]),
        Some(arena.decl("Model", a))
    );
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse_modules(&arena, &[("barrel.ts", "export { Model } from './b';")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut incremental = Compilation::build(&changed, &changed_ids, Arc::clone(&arena));
    incremental.ingest_from_db(db.conn());
    assert_eq!(
        incremental.return_type_id_of(create),
        Some(arena.decl("Model", b))
    );
    assert_eq!(
        incremental
            .generic_return_of(create)
            .unwrap()
            .instantiate(&arena, &[arena.intern(Type::Unknown)]),
        Some(arena.decl("Model", b)),
        "generic templates must be captured after imported signature rebinding"
    );
    incremental.persist_type_info(db.conn()).unwrap();
    let mut reloaded = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    reloaded.ingest_from_db(db.conn());
    assert_eq!(
        reloaded.return_type_id_of(create),
        Some(arena.decl("Model", b))
    );
    assert_eq!(
        reloaded
            .generic_return_of(create)
            .unwrap()
            .instantiate(&arena, &[arena.intern(Type::Unknown)]),
        Some(arena.decl("Model", b))
    );
}

#[test]
fn empty_type_batch_preserves_existing_id_metadata() {
    let arena = Arc::new(TypeArena::new());
    let ty = arena.decl("Same", 7);
    let mut tree = Compilation::build(&[], &SymbolIds::default(), arena);
    tree.type_info_by_id.entry(3).or_default().return_type_id = Some(ty);
    tree.capture_lexical_types(&[], &SymbolIds::default());
    assert_eq!(tree.return_type_id_of(3), Some(ty));
}

#[test]
fn canonical_type_metadata_round_trips_with_alias_and_generic_owner_ids() {
    let db = crate::Database::open_in_memory().unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO files (id,path,hash,language,last_indexed)
        VALUES (1,'a.ts','a','typescript',0);
        INSERT INTO symbols (id,file_id,name,qualified_name,kind,line,col)
        VALUES (10,1,'View','View','type_alias',0,0);",
        )
        .unwrap();
    let arena = Arc::new(TypeArena::new());
    let argument = arena.decl("Model", 71);
    let parameter = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 35,
        bound: None,
    });
    let ret = arena.intern(Type::Generic { param: parameter });
    let mut tree = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    let info = tree.type_info_by_id.entry(10).or_default();
    info.generic_param_ids = vec![parameter];
    info.generic_param_default_ids = vec![Some(argument)];
    let callback = arena.intern(Type::Function {
        params: vec![argument, ret],
        return_: arena.intern(Type::Unknown),
    });
    info.parameter_type_ids = Some(vec![callback]);
    info.lexical_alias = Some(
        super::super::super::contract::generic_return::GenericReturn::bound(
            vec![parameter],
            vec![Some(argument)],
            ret,
        ),
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored_arena = Arc::new(TypeArena::new());
    let snapshot: String = db
        .conn()
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    restored_arena.restore_snapshot(&snapshot);
    let mut loaded = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored_arena));
    loaded.ingest_from_db(db.conn());
    let loaded_info = &loaded.type_info_by_id[&10];
    assert_eq!(loaded_info.generic_param_ids, [parameter]);
    assert_eq!(loaded_info.generic_param_default_ids, [Some(argument)]);
    assert_eq!(loaded_info.parameter_type_ids, Some(vec![callback]));
    assert_eq!(
        super::super::super::generics::param_patterns(
            &loaded,
            &restored_arena,
            loaded.symbol_by_id(10).unwrap()
        ),
        vec![callback],
        "canonical parameters do not need a signature string"
    );
    assert_eq!(
        restored_arena.get(callback),
        Type::Function {
            params: vec![argument, ret],
            return_: restored_arena.intern(Type::Unknown)
        }
    );
    assert_eq!(
        loaded_info
            .lexical_alias
            .as_ref()
            .unwrap()
            .instantiate(&restored_arena, &[]),
        Some(argument)
    );
    assert_eq!(
        restored_arena.generic_param(parameter).owner_symbol_index,
        35
    );
    loaded.type_info_by_id.get_mut(&10).unwrap().lexical_alias = None;
    loaded
        .type_info_by_id
        .get_mut(&10)
        .unwrap()
        .parameter_type_ids = Some(vec![]);
    loaded
        .load_lexical_type_info(db.conn(), &[10].into_iter().collect())
        .unwrap();
    assert!(
        loaded.type_info_by_id[&10].lexical_alias.is_none(),
        "a fresh parse wins over persisted aliases"
    );
    assert_eq!(loaded.type_info_by_id[&10].parameter_type_ids, Some(vec![]));
    db.conn()
        .execute("DELETE FROM symbols WHERE id=10", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored_arena);
    deleted.ingest_from_db(db.conn());
    assert!(
        !deleted.type_info_by_id.contains_key(&10),
        "snapshot metadata cannot resurrect a deleted row"
    );
}

#[test]
fn snapshots_without_source_parameters_remain_explicitly_legacy() {
    let info = TypeInfo::default();
    let mut payload = serde_json::to_value(&info).unwrap();
    payload
        .as_object_mut()
        .unwrap()
        .remove("parameter_type_ids");
    let restored: TypeInfo = serde_json::from_value(payload).unwrap();
    assert!(restored.parameter_type_ids.is_none());
}
