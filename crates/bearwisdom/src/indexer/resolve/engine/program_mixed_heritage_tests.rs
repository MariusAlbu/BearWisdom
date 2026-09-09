use super::super::super::tests::{context, owner, parse};
use super::*;
use std::{collections::HashSet, sync::Arc};

#[test]
fn mixed_refinement_does_not_mutate_shared_parent_or_sibling_values() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", "export {}; interface Base<T> { readonly value?: T; read<U extends T>(first: U, next?: U): U; } type RequiredValue<T> = Base<T> & { value: {} }; interface Child<T extends object> extends RequiredValue<T> {} interface Sibling<T> extends Base<T> {}")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let child = owner(&ids, &files[0], "Child");
    let sibling = owner(&ids, &files[0], "Sibling");
    let base = owner(&ids, &files[0], "Base");
    let value = owner(&ids, &files[0], "value");
    let read = owner(&ids, &files[0], "read");
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert!(lookup.symbol_by_id(child).is_some());
        let projected = lookup.effective_member(child, value).unwrap();
        let unchanged = lookup.effective_member(sibling, value).unwrap();
        assert!(!projected.optional);
        assert!(!projected.readonly);
        assert!(unchanged.optional);
        assert!(unchanged.readonly);
        assert_eq!(projected.origin.declaration, Some(value));
        assert_eq!(projected.origin.source, unchanged.origin.source);
        assert_eq!(projected.origin.signature, unchanged.origin.signature);
        let arena = tree.type_arena().unwrap();
        let parameter = lookup.canonical_type_info(child).unwrap().generic_param_ids[0];
        assert_eq!(
            projected.info.field_type_id,
            Some(arena.generic_type(parameter))
        );
        assert_ne!(lookup.field_type_id_of(value), projected.info.field_type_id);
        assert_eq!(lookup.parent_class_ids(child), vec![base]);
        assert!(matches!(
            arena.get(
                lookup
                    .canonical_type_info(child)
                    .unwrap()
                    .base_type_id
                    .unwrap()
            ),
            Type::Intersection(_)
        ));
        let callable = lookup.effective_member(child, read).unwrap();
        assert_eq!(callable.signature.generic_parameters.len(), 1);
        assert!(callable.signature.syntax.parameters[1].optional);
        assert_eq!(
            callable.signature.constraints,
            vec![Some(arena.generic_type(parameter))]
        );
        let selected: &dyn SymbolLookup = &lookup;
        let concrete = arena.intern(Type::Operator(TypeOperator::Object(vec![])));
        let receiver = arena.intern(Type::Apply {
            base: selected.declaration_type(arena, child).unwrap(),
            args: vec![concrete],
        });
        assert_eq!(
            selected.projected_member_yield(arena, receiver, value, false),
            Some(Some(concrete))
        );
        let relation = types::Relation {
            lookup: &lookup,
            arena,
        };
        let key = arena.intern(Type::Literal(
            crate::type_checker::core::types::LitValue::Str("value".into()),
        ));
        let indexed = arena.intern(Type::Operator(TypeOperator::IndexedAccess {
            object: receiver,
            index: key,
        }));
        assert_eq!(
            relation.canonical(indexed, 0),
            Some(concrete),
            "type queries must see the same required refinement as member reads"
        );
        let nominal = lookup.nominal_surface(child).unwrap();
        let member = nominal
            .members
            .iter()
            .find(|m| m.origin.declaration == Some(value))
            .unwrap();
        assert!(!member.property.optional);
        assert!(!member.property.readonly);
        let signature = selected.member_info(arena, receiver, read).unwrap();
        let bindings = crate::indexer::resolve::engine::bound_call::environment(
            selected,
            arena,
            selected.symbol_by_id(read).unwrap(),
            receiver,
            Some(child),
            &[concrete],
            &[concrete],
        )
        .unwrap();
        assert_eq!(
            substitute(arena, signature.return_type_id.unwrap(), &bindings),
            concrete
        );
        let foreign = arena.decl_in(
            crate::type_checker::core::types::NominalContextId::fresh(),
            "Child",
            child,
        );
        assert!(selected.member_info(arena, foreign, read).is_none());
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(
        &[],
        &crate::indexer::symbol_ids::SymbolIds::default(),
        restored,
    );
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn mixed_origins_and_values_follow_barrel_retarget_edit_and_provider_deletion() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("left.ts", "export interface Root { value?: string; }"),
        ("right.ts", "export interface Root { value?: number; }"),
        ("barrel.ts", "export type { Root } from './left';"),
        ("child.ts", "import type { Root } from './barrel'; type RequiredRoot = Root & { value: {} }; export interface Child extends RequiredRoot {}")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let child = owner(&ids, &files[3], "Child");
    let left = owner(&ids, &files[0], "value");
    let right = owner(&ids, &files[1], "value");
    let check = |tree: &Compilation, row, kind| {
        let lookup = tree.program_lookup("child.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let fact = lookup.effective_member(child, row).unwrap();
        assert_eq!(fact.origin.declaration, Some(row));
        assert!(!fact.optional);
        assert_eq!(
            fact.info.field_type_id,
            Some(arena.intern(Type::Intrinsic(kind)))
        );
        let name = lookup.member_index().unwrap().name("value").unwrap();
        assert_eq!(
            crate::indexer::resolve::engine::member_selection::select(
                &lookup,
                child,
                name,
                &|_| true
            ),
            crate::indexer::resolve::engine::member_selection::Selection::Unique(row)
        );
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    check(&tree, left, Intrinsic::String);
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
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0], &files[1], &changed[0], &files[3]])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, right, Intrinsic::Number);
    edited.persist_type_info(db.conn()).unwrap();
    let provider = parse(
        &arena,
        &[(
            "right.ts",
            "export interface Root { readonly value?: bigint; }",
        )],
    );
    let (_, provider_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &provider,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let right = owner(&provider_ids, &provider[0], "value");
    let mut edited = Compilation::build_with_context(
        &provider,
        &provider_ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0], &provider[0], &changed[0], &files[3]])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, right, Intrinsic::BigInt);
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(
        &[],
        &crate::indexer::symbol_ids::SymbolIds::default(),
        Arc::clone(&restored),
    );
    cold.ingest_from_db(db.conn());
    check(&cold, right, Intrinsic::BigInt);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build_with_context(
        &[],
        &crate::indexer::symbol_ids::SymbolIds::default(),
        restored,
        Some(&context(&[&files[0], &changed[0], &files[3]])),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    let lookup = deleted.program_lookup("child.ts").unwrap();
    assert!(lookup.symbol_by_id(left).is_some());
    assert!(lookup.symbol_by_id(child).is_none());
}

#[test]
fn portable_rowless_refinements_preserve_source_signatures_without_fabricating_rows() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let source = "function discard() {} export interface Root<T> { value?: T; } type RequiredRoot<T> = Root<T> & { value: {} }; export interface Child<T extends object> extends RequiredRoot<T> {}";
    let original = TypeArena::new();
    let mut files = parse(&original, &[("child.ts", source)]);
    let discarded = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    for property in files[0].symbols.iter_mut().filter(|s| s.name == "value") {
        property.parent_index = Some(discarded);
    }
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "value"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    arena.intern(Type::Literal(
        crate::type_checker::core::types::LitValue::Str("shifted".into()),
    ));
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
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison".into();
        symbol.signature = None;
    }
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("child.ts").unwrap();
        assert!(lookup.symbol_by_id(child).is_some());
        let facts = &lookup.view.effective_surfaces[&child];
        assert_eq!(facts.len(), 1);
        assert!(facts[0].origin.declaration.is_none());
        assert!(!facts[0].optional);
        assert_eq!(
            &source[facts[0].origin.signature.0.start as usize
                ..facts[0].origin.signature.0.end as usize],
            "value?: T"
        );
        let name = lookup.member_index().unwrap().name("value").unwrap();
        assert_eq!(
            crate::indexer::resolve::engine::member_selection::select(
                &lookup,
                child,
                name,
                &|_| true
            ),
            crate::indexer::resolve::engine::member_selection::Selection::Incomplete
        );
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(
        &[],
        &crate::indexer::symbol_ids::SymbolIds::default(),
        restored,
    );
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn mixed_work_exhaustion_is_not_memoized_as_a_semantic_rejection() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", "export {}; interface Child {}")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    let child = owner(&ids, &files[0], "Child");
    let empty = arena.intern(Type::Operator(TypeOperator::Object(vec![])));
    let shapes = [(
        child,
        Shape {
            bases: vec![empty],
            heritage: true,
            ..Default::default()
        },
    )]
    .into_iter()
    .collect();
    let mut proof = Proof {
        shapes: &shapes,
        relation: types::Relation {
            lookup: &lookup,
            arena: &arena,
        },
        memo: Default::default(),
        active: Default::default(),
        heights: vec![],
        applications: Default::default(),
        exhausted: false,
        remaining: 1,
    };
    assert!(proof.effective(child).is_none());
    assert!(proof.exhausted);
    assert!(proof.memo.is_empty());
    proof.remaining = 4096;
    proof.exhausted = false;
    assert!(proof.effective(child).is_some());
}
