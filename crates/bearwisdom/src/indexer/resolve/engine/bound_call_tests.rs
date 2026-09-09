use super::*;
use crate::type_checker::core::types::GenericParamData;

#[test]
fn inference_does_not_equate_one_physical_owner_across_program_contexts() {
    use crate::type_checker::core::types::NominalContextId;
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let (id, generic) = parameter(&arena, 1);
    let own = arena.decl_in(a, "Box", 11);
    let pattern = arena.intern(Type::Apply {
        base: own,
        args: vec![generic],
    });
    let scalar = arena.primitive(crate::type_checker::core::types::PrimKind::Bool);
    for head in [own, arena.decl_in(b, "Box", 11), arena.decl("Box", 11)] {
        let actual = arena.intern(Type::Apply {
            base: head,
            args: vec![scalar],
        });
        let mut env = Bindings::default();
        infer(
            &arena,
            pattern,
            actual,
            &[id].into_iter().collect(),
            &mut env,
            &mut FxHashSet::default(),
        );
        assert_eq!(env.get(&id), (head == own).then_some(&scalar));
    }
}

#[test]
fn call_inference_preserves_region_identity_kind_and_conflicts() {
    use crate::type_checker::core::types::Mutability;
    let arena = TypeArena::new();
    let region = |owner| {
        arena.intern_generic(GenericParamData {
            name: "'a".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: owner,
            bound: None,
        })
    };
    let p = region(1);
    let a = region(2);
    let b = region(3);
    let (t, generic) = parameter(&arena, 1);
    let nominal = arena.decl("Doc", 42);
    let reference = |region, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability: Mutability::Shared,
            inner,
        })
    };
    let pattern = reference(Lifetime::Parameter(p), generic);
    let mut env = Bindings::default();
    let mut conflicts = FxHashSet::default();
    let open = [p, t].into_iter().collect();
    infer(
        &arena,
        pattern,
        reference(Lifetime::Parameter(a), nominal),
        &open,
        &mut env,
        &mut conflicts,
    );
    assert_eq!(env[&p], arena.generic_type(a));
    assert_eq!(env[&t], nominal);
    assert!(conflicts.is_empty());
    infer(
        &arena,
        pattern,
        reference(Lifetime::Parameter(b), nominal),
        &open,
        &mut env,
        &mut conflicts,
    );
    assert!(conflicts.contains(&p));
    assert!(!conflicts.contains(&t));
    let mut empty = Bindings::default();
    infer(
        &arena,
        arena.generic_type(p),
        arena.intern(Type::Region(Lifetime::Unknown)),
        &open,
        &mut empty,
        &mut FxHashSet::default(),
    );
    infer(
        &arena,
        generic,
        arena.generic_type(a),
        &open,
        &mut empty,
        &mut FxHashSet::default(),
    );
    assert!(
        empty.is_empty(),
        "unknown regions and kind mismatches are not inferred"
    );
}

fn parameter(arena: &TypeArena, owner: usize) -> (GenericParamId, TypeId) {
    let param = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: owner,
        bound: None,
    });
    (param, arena.intern(Type::Generic { param }))
}

#[test]
fn inference_never_matches_same_display_nominals_with_distinct_declarations() {
    let arena = TypeArena::new();
    let (id, generic) = parameter(&arena, 1);
    let pattern = arena.intern(Type::Apply {
        base: arena.decl("Box", 11),
        args: vec![generic],
    });
    let actual = arena.intern(Type::Apply {
        base: arena.decl("Box", 12),
        args: vec![arena.decl("Model", 31)],
    });
    let mut env = Bindings::default();
    infer(
        &arena,
        pattern,
        actual,
        &[id].into_iter().collect(),
        &mut env,
        &mut FxHashSet::default(),
    );
    assert!(env.is_empty());
}

#[test]
fn generic_owner_identity_and_conflicting_arguments_are_not_first_winner_guesses() {
    let arena = TypeArena::new();
    let (class_id, class_t) = parameter(&arena, 1);
    let (method_id, method_t) = parameter(&arena, 2);
    let alpha = arena.decl("Model", 31);
    let beta = arena.decl("Model", 32);
    let mut env = Bindings::default();
    let mut conflicts = FxHashSet::default();
    let open = [method_id].into_iter().collect();
    infer(&arena, class_t, alpha, &open, &mut env, &mut conflicts);
    assert!(!env.contains_key(&class_id));
    infer(&arena, method_t, alpha, &open, &mut env, &mut conflicts);
    infer(&arena, method_t, beta, &open, &mut env, &mut conflicts);
    assert!(conflicts.contains(&method_id));
    assert_eq!(
        substitute(&arena, method_t, &[(class_id, alpha)].into_iter().collect()),
        method_t
    );
}

#[test]
fn anonymous_call_parameters_bind_only_callee_ids_and_do_not_shift_explicit_types() {
    use super::super::{compilation::Compilation, contract::TypeInfo, testkit::sym};
    use crate::{indexer::symbol_ids::SymbolIds, type_checker::core::types::Mutability};
    use std::sync::Arc;
    let arena = Arc::new(TypeArena::new());
    let mut lookup = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    let region = || {
        arena.intern_generic(GenericParamData {
            name: "'_".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: 0,
            bound: None,
        })
    };
    let callee_region = region();
    let caller_region = region();
    let sibling_region = region();
    let (t, generic) = parameter(&arena, 7);
    let nominal = arena.decl("Doc", 42);
    let reference = |region, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability: Mutability::Shared,
            inner,
        })
    };
    let pattern = reference(Lifetime::Parameter(callee_region), generic);
    lookup.type_info_by_id.insert(
        7,
        TypeInfo {
            generic_param_ids: vec![t],
            elided_input_params: vec![(17, 0, callee_region)],
            parameter_type_ids: Some(vec![pattern]),
            ..Default::default()
        },
    );
    let callee = sym(7, "make", "make", "function", "lib.rs");
    let unknown = arena.intern(Type::Unknown);
    let actual = reference(Lifetime::Parameter(caller_region), nominal);
    let env = environment(
        &lookup,
        &arena,
        &callee,
        unknown,
        None,
        &[nominal],
        &[actual],
    )
    .unwrap();
    assert_eq!(env[&t], nominal);
    assert_eq!(env[&callee_region], arena.generic_type(caller_region));
    assert!(!env.contains_key(&caller_region));
    assert!(!env.contains_key(&sibling_region));
    assert_eq!(substitute(&arena, pattern, &env), actual);
    let env = environment(&lookup, &arena, &callee, unknown, None, &[], &[unknown]).unwrap();
    assert_eq!(
        arena.get(env[&callee_region]),
        Type::Region(Lifetime::Unknown)
    );
    assert!(!env.contains_key(&t));
}
