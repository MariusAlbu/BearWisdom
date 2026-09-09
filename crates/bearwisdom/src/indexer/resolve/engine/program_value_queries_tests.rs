use super::*;

#[test]
fn configured_constructor_initializer_uses_runtime_signature_not_same_spelled_type() {
    use super::super::merge_proof::tests::{context, owner, parse};
    use std::{collections::HashSet, sync::Arc};
    let arena = Arc::new(TypeArena::new());
    let source = "export {}; interface Result<T> { read(): T; } interface Factory { new<T>(): Result<T>; } declare const Build: Factory; interface Build<T> { wrong(): T; } class Holder<T> { value = new Build<T>(); }";
    let files = parse(&arena, &[("main.ts", source)]);
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
    let value = lookup
        .field_type_id_of(owner(&ids, &files[0], "value"))
        .expect("initializer-derived field type");
    let Type::Apply { base, args } = arena.get(value) else {
        panic!("initializer must retain its generic result application")
    };
    assert_eq!(
        crate::indexer::resolve::engine::head_decl::head_decl_id(&arena, base),
        Some(owner(&ids, &files[0], "Result"))
    );
    let holder = lookup
        .canonical_type_info(owner(&ids, &files[0], "Holder"))
        .unwrap();
    assert_eq!(args, vec![arena.generic_type(holder.generic_param_ids[0])]);
}

#[test]
fn proved_repeated_property_feeds_type_queries_and_computed_key_cascades() {
    use super::super::merge_proof::tests::{context, owner, parse};
    use std::{collections::HashSet, sync::Arc};
    let arena = Arc::new(TypeArena::new());
    let source = "declare const token: unique symbol; interface Keys { tag: typeof token; } interface Keys { tag: typeof token; } declare const keys: Keys; type Tag = typeof keys.tag; declare const selected: Tag; interface Uses { [selected](): void; } interface Uses { keep(): void; }";
    let files = parse(&arena, &[("main.ts", source)]);
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
    assert!(lookup
        .symbol_by_id(owner(&ids, &files[0], "Keys"))
        .is_some());
    let start = source.find("[selected]").unwrap() as u32;
    assert!(
        lookup
            .computed_key(SourceSpan {
                start,
                end: start + 10
            })
            .flatten()
            .is_some(),
        "a proved duplicate property must feed the entire alias/value/key cascade"
    );
    assert!(lookup
        .symbol_by_id(owner(&ids, &files[0], "Uses"))
        .is_some());
}

#[test]
fn dependency_keys_separate_fields_from_alias_types() {
    let arena = TypeArena::new();
    let ty = arena.intern(Type::Unknown);
    assert_ne!(Key::Field(1), Key::Alias(ty));
}

fn solver<'a>(view: &'a View, tree: &'a Compilation, arena: &'a TypeArena) -> Solver<'a> {
    Solver {
        view,
        tree,
        arena,
        values: Values::default(),
        sources: Default::default(),
        fields: Default::default(),
        aliases: Default::default(),
        queries: Default::default(),
        signatures: Default::default(),
        bases: Default::default(),
        interfaces: Default::default(),
        initializers: Default::default(),
        constructor_calls: Default::default(),
        memo: Default::default(),
        heights: Default::default(),
        exhausted: Cell::new(0),
    }
}

fn chain(solver: &Solver, remaining: i64) -> Option<TypeId> {
    solver.memoized(Key::Field(remaining), || {
        if remaining == 0 {
            Some(solver.arena.intern(Type::Intrinsic(
                crate::type_checker::core::types::Intrinsic::String,
            )))
        } else {
            chain(solver, remaining - 1)
        }
    })
}

#[test]
fn memoized_dependency_depth_does_not_depend_on_warmup_order() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let view = View::empty();
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    for warm_first in [false, true] {
        let solver = solver(&view, &tree, &arena);
        if warm_first {
            assert!(chain(&solver, 80).is_some());
        }
        assert!(chain(&solver, 200).is_none());
        assert!(chain(&solver, 80).is_some());
        assert!(chain(&solver, 127).is_some());
        assert!(chain(&solver, 128).is_none());
        assert!(chain(&solver, 200).is_none());
        assert!(!solver
            .memo
            .borrow()
            .values()
            .any(|state| matches!(state, State::Active)));
    }
}

#[test]
fn memoized_dependency_cycles_do_not_publish_partial_values() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let view = View::empty();
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    let solver = solver(&view, &tree, &arena);
    assert_eq!(
        solver.memoized(Key::Field(1), || solver.memoized(Key::Field(2), || solver
            .memoized(Key::Field(1), || panic!("active cycle must not evaluate")))),
        None
    );
    assert_eq!(
        solver.memoized(Key::Field(1), || panic!(
            "cached cycle must stay unresolved"
        )),
        None
    );
    assert_eq!(
        solver.memoized(Key::Field(2), || panic!(
            "cached cycle must stay unresolved"
        )),
        None
    );
    assert!(solver.heights.borrow().is_empty());
}
