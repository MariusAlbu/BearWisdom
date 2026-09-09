use super::*;
use crate::type_checker::core::types::GenericParamData;

#[test]
fn regions_substitute_by_id_and_argument_kinds_cannot_cross() {
    use crate::type_checker::core::types::Mutability;
    let arena = TypeArena::new();
    let p = arena.intern_generic(GenericParamData {
        name: "'a".into(),
        kind: GenericParamKind::Lifetime,
        owner_symbol_index: 1,
        bound: None,
    });
    let foreign = arena.intern_generic(GenericParamData {
        name: "'a".into(),
        kind: GenericParamKind::Lifetime,
        owner_symbol_index: 2,
        bound: None,
    });
    let t = parameter(&arena, "T", 1);
    let nominal = arena.decl("Doc", 42);
    let reference = |region, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability: Mutability::Shared,
            inner,
        })
    };
    let template = GenericReturn::bound(
        vec![p, t],
        vec![],
        reference(Lifetime::Parameter(p), arena.generic_type(t)),
    );
    let arg = arena.generic_type(foreign);
    let static_arg = arena.intern(Type::Region(Lifetime::Static));
    assert_eq!(
        template.instantiate(&arena, &[arg, nominal]),
        Some(reference(Lifetime::Parameter(foreign), nominal))
    );
    assert_eq!(
        template.instantiate(&arena, &[static_arg, nominal]),
        Some(reference(Lifetime::Static, nominal))
    );
    assert_eq!(template.instantiate(&arena, &[nominal, arg]), None);
    assert_eq!(template.instantiate(&arena, &[arg, arg]), None);
    let env = [(p, static_arg)].into_iter().collect();
    assert_eq!(
        substitute(&arena, arg, &env),
        arg,
        "same spelling does not capture another owner's parameter"
    );
    let c = arena.intern_generic(GenericParamData {
        name: "N".into(),
        kind: GenericParamKind::Const,
        owner_symbol_index: 1,
        bound: None,
    });
    assert_eq!(
        GenericReturn::bound(vec![c], vec![], nominal).instantiate(&arena, &[nominal]),
        None
    );
}

fn parameter(arena: &TypeArena, name: &str, owner: usize) -> GenericParamId {
    arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: name.into(),
        owner_symbol_index: owner,
        bound: None,
    })
}

#[test]
fn substitution_uses_parameter_identity_and_never_nominal_spelling() {
    let arena = TypeArena::new();
    let own = parameter(&arena, "T", 1);
    let foreign = parameter(&arena, "T", 2);
    let nominal = arena.decl("T", 91);
    let foreign_ty = arena.intern(Type::Generic { param: foreign });
    let result = arena.intern(Type::Tuple(vec![arena.class("T"), foreign_ty, nominal]));
    let template = GenericReturn::capture(&arena, &[own], &[None], result);
    let actual = arena.decl("Actual", 92);
    assert_eq!(
        arena.get(template.instantiate(&arena, &[actual]).unwrap()),
        Type::Tuple(vec![actual, foreign_ty, nominal])
    );
    let bindings = [(own, actual)].into_iter().collect();
    let unbound_nominal = arena.class("T");
    assert_eq!(
        substitute(&arena, unbound_nominal, &bindings),
        unbound_nominal
    );
}

#[test]
fn defaults_are_canonicalized_and_expanded_in_parameter_order() {
    let arena = TypeArena::new();
    let t = parameter(&arena, "T", 1);
    let u = parameter(&arena, "U", 1);
    let result = arena.intern_type_str("Promise<U[]>");
    let template = GenericReturn::capture(&arena, &[t, u], &[None, Some(arena.class("T"))], result);
    let alpha = arena.decl("Alpha", 7);
    // Use the same parser's wrapper representation, then verify the leaf by ID.
    let expected = arena.rebind_class_params(result, &[("U".into(), alpha)].into_iter().collect());
    assert_eq!(template.instantiate(&arena, &[alpha]), Some(expected));
    assert_eq!(template.instantiate(&arena, &[]), None);
    assert_eq!(template.instantiate(&arena, &[alpha, alpha, alpha]), None);
}

#[test]
fn nested_structural_types_rewrite_without_erasing_their_shape() {
    let arena = TypeArena::new();
    let t = parameter(&arena, "T", 1);
    let param = arena.intern(Type::Generic { param: t });
    let actual = arena.decl("A", 3);
    let wrap = |leaf| {
        arena.intern(Type::Function {
            params: vec![
                arena.intern(Type::Indirect {
                    kind: crate::type_checker::core::types::Indirection::Reference(
                        crate::type_checker::core::types::Lifetime::Static,
                    ),
                    mutability: crate::type_checker::core::types::Mutability::Mutable,
                    inner: leaf,
                }),
                arena.intern(Type::Optional(leaf)),
                arena.intern(Type::Constructor(leaf)),
            ],
            return_: arena.intern(Type::AsyncWrapper(arena.intern(Type::Iterator(
                arena.intern(Type::Intersection(vec![
                    arena.intern(Type::Union(vec![leaf])),
                ])),
            )))),
        })
    };
    assert_eq!(
        substitute(&arena, wrap(param), &[(t, actual)].into_iter().collect()),
        wrap(actual)
    );
}

#[test]
fn recapture_replaces_templates_and_drops_removed_generic_metadata() {
    let arena = TypeArena::new();
    let t = parameter(&arena, "T", 1);
    let mut slots = FxHashMap::default();
    let mut info = super::super::TypeInfo::default();
    info.generic_param_ids = vec![t];
    info.return_type_id = Some(arena.class("T"));
    slots.insert(9, info);
    capture_all(&arena, &mut slots);
    let actual = arena.decl("A", 42);
    assert_eq!(
        slots[&9]
            .generic_return
            .as_ref()
            .unwrap()
            .instantiate(&arena, &[actual]),
        Some(actual)
    );
    slots.get_mut(&9).unwrap().generic_param_ids.clear();
    capture_all(&arena, &mut slots);
    assert!(slots[&9].generic_return.is_none());
}

#[test]
fn substitution_preserves_conditional_pattern_binders_and_true_branch_references() {
    use crate::type_checker::core::types::{Intrinsic, TypeOperator};
    let arena = TypeArena::new();
    let outer = parameter(&arena, "T", 1);
    let inner = parameter(&arena, "T", 2);
    let value = arena.intern(Type::Intrinsic(Intrinsic::String));
    let outer_type = arena.generic_type(outer);
    let inner_type = arena.generic_type(inner);
    let pattern = arena.intern(Type::Operator(TypeOperator::Infer(inner_type)));
    let conditional = arena.intern(Type::Operator(TypeOperator::Conditional {
        check: outer_type,
        extends: pattern,
        when_true: inner_type,
        when_false: outer_type,
        distributive: Some(true),
    }));
    let result = substitute(
        &arena,
        conditional,
        &[(outer, value), (inner, value)].into_iter().collect(),
    );
    assert_eq!(
        arena.get(result),
        Type::Operator(TypeOperator::Conditional {
            check: value,
            extends: pattern,
            when_true: inner_type,
            when_false: value,
            distributive: Some(true),
        })
    );
    let restored = TypeArena::new();
    restored.restore_snapshot(&arena.serialize_snapshot());
    assert_eq!(restored.get(pattern), arena.get(pattern));
}
