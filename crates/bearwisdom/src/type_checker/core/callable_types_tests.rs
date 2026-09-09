use super::super::{Intrinsic, Type};
use super::*;

#[test]
fn callable_origin_and_all_operands_survive_mapping() {
    let site = SourceSpan { start: 10, end: 90 };
    let c = Callable {
        origin: site,
        generics: vec![CallableGeneric {
            parameter: 1,
            constraint: Some(2),
            default: Some(3),
        }],
        parameters: vec![CallableParameter {
            declaration: site,
            ty: 4,
            optional: true,
            rest: false,
            receiver: false,
        }],
        result: 5,
        predicate: Some(CallablePredicate {
            parameter: site,
            asserted: Some(6),
            asserts: false,
        }),
        complete: true,
    };
    let mapped = c.map(
        CallableOrigin::new(NominalContextId::fresh(), 4, site),
        |id| id + 10,
    );
    assert_eq!(
        mapped.operands().copied().collect::<Vec<_>>(),
        vec![15, 14, 11, 12, 13, 16]
    );
    assert_eq!(mapped.predicate.unwrap().parameter, site);
}

#[test]
fn scalar_only_callable_is_program_scoped_and_snapshot_context_is_fresh() {
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let result = arena.intern(Type::Intrinsic(Intrinsic::Boolean));
    let c = Callable {
        origin: CallableOrigin::new(a, 0, SourceSpan { start: 0, end: 10 }),
        generics: vec![],
        parameters: vec![],
        result,
        predicate: None,
        complete: true,
    };
    let id = arena.intern(Type::Callable(Box::new(c.clone())));
    assert!(arena.accepts_nominal_context(id, Some(a)));
    assert!(!arena.accepts_nominal_context(id, Some(b)));
    assert!(!arena.accepts_nominal_context(id, None));
    assert_ne!(
        id,
        arena.intern(Type::Callable(Box::new(
            c.map(CallableOrigin::new(a, 1, c.origin.signature), |t| *t)
        )))
    );
    let restored = TypeArena::new();
    restored.restore_snapshot(&arena.serialize_snapshot());
    assert!(!restored.accepts_nominal_context(id, Some(a)));
    let Type::Callable(restored_type) = restored.get(id) else {
        panic!()
    };
    assert_eq!(restored_type.origin.signature, c.origin.signature);
}

#[test]
fn substitution_protects_callable_binders_and_snapshot_merge_remaps_every_operand() {
    use super::super::{GenericParamData, GenericParamKind};
    use crate::indexer::resolve::engine::contract::generic_return::substitute;
    let arena = TypeArena::new();
    let parameter = || {
        arena.intern_generic(GenericParamData {
            name: "same".into(),
            kind: GenericParamKind::Type,
            owner_symbol_index: 0,
            bound: None,
        })
    };
    let outer = parameter();
    let inner = parameter();
    let outer_ty = arena.generic_type(outer);
    let inner_ty = arena.generic_type(inner);
    let scalar = arena.intern(Type::Intrinsic(Intrinsic::String));
    let site = SourceSpan { start: 20, end: 25 };
    let c = Callable {
        origin: CallableOrigin::new(
            NominalContextId::fresh(),
            3,
            SourceSpan { start: 0, end: 60 },
        ),
        generics: vec![CallableGeneric {
            parameter: inner_ty,
            constraint: Some(outer_ty),
            default: Some(outer_ty),
        }],
        parameters: vec![CallableParameter {
            declaration: site,
            ty: inner_ty,
            optional: false,
            rest: false,
            receiver: false,
        }],
        result: outer_ty,
        predicate: Some(CallablePredicate {
            parameter: site,
            asserted: Some(inner_ty),
            asserts: false,
        }),
        complete: true,
    };
    let id = arena.intern(Type::Callable(Box::new(c.clone())));
    let changed = substitute(
        &arena,
        id,
        &[(inner, scalar), (outer, scalar)].into_iter().collect(),
    );
    let Type::Callable(c) = arena.get(changed) else {
        panic!()
    };
    assert_eq!(c.generics[0].parameter, inner_ty);
    assert_eq!(c.parameters[0].ty, inner_ty);
    assert_eq!(c.predicate.as_ref().unwrap().asserted, Some(inner_ty));
    assert_eq!(c.result, scalar);
    assert_eq!(
        (c.generics[0].constraint, c.generics[0].default),
        (Some(scalar), Some(scalar))
    );
    let destination = TypeArena::new();
    destination.intern(Type::Literal(super::super::LitValue::Int(99)));
    let remap = crate::type_checker::core::arena_merge::merge_snapshot_into(
        &arena.serialize_snapshot(),
        &destination,
        &|id| id,
    )
    .unwrap();
    let Type::Callable(merged) = destination.get(remap.type_id(changed).unwrap()) else {
        panic!()
    };
    assert_ne!(merged.origin.context, c.origin.context);
    assert_eq!(merged.parameters[0].ty, merged.generics[0].parameter);
    assert_eq!(
        merged.predicate.unwrap().asserted,
        Some(merged.generics[0].parameter)
    );
    assert_eq!(
        destination.get(merged.result),
        Type::Intrinsic(Intrinsic::String)
    );
}
