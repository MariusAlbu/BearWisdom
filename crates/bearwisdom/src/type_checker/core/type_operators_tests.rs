use super::*;
use crate::type_checker::core::types::{
    GenericParamData, GenericParamKind, Intrinsic, NominalContextId, Type,
};

#[test]
fn operator_mapping_visits_every_operand_and_preserves_distribution_evidence() {
    let shapes = [
        TypeOperator::KeyOf(1),
        TypeOperator::Readonly(1),
        TypeOperator::Infer(1),
        TypeOperator::IndexedAccess {
            object: 1,
            index: 2,
        },
        TypeOperator::Conditional {
            check: 1,
            extends: 2,
            when_true: 3,
            when_false: 4,
            distributive: Some(true),
        },
        TypeOperator::Object(vec![
            TypeProperty {
                key: 1,
                value: 2,
                optional: true,
                readonly: true,
                index: false,
            },
            TypeProperty {
                key: 3,
                value: 4,
                optional: false,
                readonly: false,
                index: true,
            },
        ]),
        TypeOperator::Mapped {
            parameter: 1,
            keys: 2,
            remap: Some(3),
            value: 4,
            optional: MappedModifier::Remove,
            readonly: MappedModifier::Add,
        },
    ];
    for shape in shapes {
        let mut seen = vec![];
        let mapped = shape.map(|n| {
            seen.push(*n);
            n + 10
        });
        assert_eq!(seen, shape.operands().copied().collect::<Vec<_>>());
        assert_eq!(
            mapped.operands().copied().collect::<Vec<_>>(),
            seen.iter().map(|n| n + 10).collect::<Vec<_>>()
        );
        if let TypeOperator::Conditional { distributive, .. } = mapped {
            assert_eq!(distributive, Some(true));
        }
    }
}

#[test]
fn operators_preserve_nominal_context_and_cannot_hide_foreign_operands() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let left = arena.decl_in(a, "display", 10);
    let right = arena.decl_in(b, "display", 10);
    let shape = TypeOperator::Conditional {
        check: left,
        extends: left,
        when_true: left,
        when_false: left,
        distributive: None,
    };
    let id = arena.intern(Type::Operator(shape.clone()));
    assert!(arena.accepts_nominal_context(id, Some(a)));
    assert!(!arena.accepts_nominal_context(id, Some(b)));
    assert_eq!(id, arena.intern(Type::Operator(shape.clone())));
    for position in 0..4 {
        let mut i = 0;
        let mixed = shape.map(|operand| {
            let result = if i == position { right } else { *operand };
            i += 1;
            result
        });
        let mixed = arena.intern(Type::Operator(mixed));
        assert!(!arena.accepts_nominal_context(mixed, Some(a)));
        assert!(!arena.accepts_nominal_context(mixed, Some(b)));
        assert!(!arena.accepts_nominal_context(mixed, None));
    }
}

#[test]
fn operator_substitution_and_snapshot_merge_keep_generic_identity() {
    let arena = TypeArena::new();
    let param = arena.intern_generic(GenericParamData {
        name: "poisoned display".into(),
        owner_symbol_index: 7,
        kind: GenericParamKind::Type,
        bound: None,
    });
    let generic = arena.generic_type(param);
    let atom = arena.intern(Type::Intrinsic(Intrinsic::String));
    let inner = arena.intern(Type::Operator(TypeOperator::KeyOf(generic)));
    let shape = TypeOperator::Conditional {
        check: generic,
        extends: inner,
        when_true: atom,
        when_false: generic,
        distributive: Some(true),
    };
    let id = arena.intern(Type::Operator(shape));
    let rewritten = crate::indexer::resolve::engine::contract::generic_return::substitute(
        &arena,
        id,
        &[(param, atom)].into_iter().collect(),
    );
    let Type::Operator(TypeOperator::Conditional {
        check,
        extends,
        when_true,
        when_false,
        distributive,
    }) = arena.get(rewritten)
    else {
        panic!()
    };
    assert_eq!((check, when_true, when_false), (atom, atom, atom));
    assert_eq!(distributive, Some(true));
    assert_eq!(
        arena.get(extends),
        Type::Operator(TypeOperator::KeyOf(atom))
    );
    let restored = TypeArena::new();
    restored.restore_snapshot(&arena.serialize_snapshot());
    assert_eq!(restored.get(id), arena.get(id));
    let dst = TypeArena::new();
    dst.intern(Type::Unknown);
    let remap = crate::type_checker::core::arena_merge::merge_snapshot_into(
        &arena.serialize_snapshot(),
        &dst,
        &|row| row + 100,
    )
    .unwrap();
    let expected = match arena.get(id) {
        Type::Operator(op) => Type::Operator(op.map(|t| remap.type_id(*t).unwrap())),
        _ => panic!(),
    };
    assert_eq!(dst.get(remap.type_id(id).unwrap()), expected);
}

#[test]
fn structural_substitution_does_not_replace_a_bound_mapped_parameter() {
    let arena = TypeArena::new();
    let parameter = |owner| {
        arena.intern_generic(GenericParamData {
            name: "same poisoned display".into(),
            kind: GenericParamKind::Type,
            owner_symbol_index: owner,
            bound: None,
        })
    };
    let outer = parameter(1);
    let inner = parameter(2);
    let key = arena.intern(Type::Literal(super::super::LitValue::Str("item".into())));
    let value = arena.intern(Type::Operator(TypeOperator::IndexedAccess {
        object: arena.generic_type(outer),
        index: arena.generic_type(inner),
    }));
    let mapped = arena.intern(Type::Operator(TypeOperator::Mapped {
        parameter: arena.generic_type(inner),
        keys: key,
        remap: Some(arena.generic_type(inner)),
        value,
        optional: MappedModifier::Remove,
        readonly: MappedModifier::Add,
    }));
    let object = arena.intern(Type::Operator(TypeOperator::Object(vec![TypeProperty {
        key,
        value: key,
        optional: true,
        readonly: false,
        index: false,
    }])));
    let rewritten = crate::indexer::resolve::engine::contract::generic_return::substitute(
        &arena,
        mapped,
        &[(outer, object), (inner, key)].into_iter().collect(),
    );
    let Type::Operator(TypeOperator::Mapped {
        parameter,
        value,
        remap,
        optional,
        readonly,
        ..
    }) = arena.get(rewritten)
    else {
        panic!()
    };
    assert_eq!(parameter, arena.generic_type(inner));
    assert_eq!(remap, Some(parameter));
    assert_eq!(
        (optional, readonly),
        (MappedModifier::Remove, MappedModifier::Add)
    );
    assert_eq!(
        arena.get(value),
        Type::Operator(TypeOperator::IndexedAccess {
            object,
            index: parameter
        })
    );
}

#[test]
fn structural_operands_preserve_context_snapshot_remapping_and_metadata() {
    let arena = TypeArena::new();
    let context = NominalContextId::fresh();
    let foreign = NominalContextId::fresh();
    let local = arena.decl_in(context, "display", 4);
    let alien = arena.decl_in(foreign, "display", 4);
    let parameter = arena.intern_generic(GenericParamData {
        name: "poison".into(),
        owner_symbol_index: 2,
        kind: GenericParamKind::Type,
        bound: None,
    });
    let shapes = [
        TypeOperator::Object(vec![
            TypeProperty {
                key: local,
                value: local,
                optional: true,
                readonly: true,
                index: false,
            },
            TypeProperty {
                key: local,
                value: local,
                optional: false,
                readonly: false,
                index: true,
            },
        ]),
        TypeOperator::Mapped {
            parameter: arena.generic_type(parameter),
            keys: local,
            remap: Some(local),
            value: local,
            optional: MappedModifier::Add,
            readonly: MappedModifier::Remove,
        },
    ];
    for shape in shapes {
        let ty = arena.intern(Type::Operator(shape.clone()));
        assert!(arena.accepts_nominal_context(ty, Some(context)));
        assert!(!arena.accepts_nominal_context(ty, Some(foreign)));
        for position in 0..shape.operands().count() {
            let mut index = 0;
            let mixed = shape.map(|id| {
                let value = if position == index { alien } else { *id };
                index += 1;
                value
            });
            assert!(
                !arena.accepts_nominal_context(arena.intern(Type::Operator(mixed)), Some(context))
            );
        }
        let destination = TypeArena::new();
        destination.intern(Type::Unknown);
        let remap = crate::type_checker::core::arena_merge::merge_snapshot_into(
            &arena.serialize_snapshot(),
            &destination,
            &|row| row + 100,
        )
        .unwrap();
        assert_eq!(
            destination.get(remap.type_id(ty).unwrap()),
            Type::Operator(shape.map(|id| remap.type_id(*id).unwrap()))
        );
        let restored = TypeArena::new();
        assert!(restored.restore_snapshot(&arena.serialize_snapshot()) > 0);
        let Type::Operator(op) = restored.get(ty) else {
            panic!()
        };
        assert_eq!(op, shape);
        assert!(
            !restored.accepts_nominal_context(ty, Some(context)),
            "restoration remints configured contexts through object/mapped operands"
        );
    }
}
