use super::super::tests::with_relation;
use super::*;

#[test]
fn normalization_never_discards_unresolved_operands() {
    with_relation(|relation| {
        let arena = relation.arena;
        let unknown = arena.intern(Type::Unknown);
        let never = arena.intern(Type::Intrinsic(Intrinsic::Never));
        for ty in [
            Type::Union(vec![unknown, never]),
            Type::Intersection(vec![unknown, never]),
            Type::Operator(TypeOperator::Object(vec![TypeProperty {
                key: arena.intern(Type::Literal(LitValue::Str("a".into()))),
                value: unknown,
                optional: true,
                readonly: false,
                index: false,
            }])),
        ] {
            let ty = arena.intern(ty);
            assert_eq!(relation.canonical(ty, 0), None);
            assert!(!relation.equal(ty, ty));
        }
    });
}

#[test]
fn evaluator_bounds_depth_and_total_branch_work() {
    with_relation(|relation| {
        let string = relation.arena.intern(Type::Intrinsic(Intrinsic::String));
        let mut nested = string;
        for _ in 0..70 {
            nested = relation.arena.intern(Type::Tuple(vec![nested]));
        }
        assert_eq!(relation.canonical(nested, 0), None);
        assert_eq!(
            relation.canonical(relation.arena.intern(Type::Tuple(vec![string; 5000])), 0),
            None
        );
        assert_eq!(relation.canonical(string, 0), Some(string));
    });
}

#[test]
fn foreign_context_or_unowned_generic_cannot_certify_nested_identity() {
    use crate::type_checker::core::types::{GenericParamData, NominalContextId};
    with_relation(|relation| {
        let arena = relation.arena;
        let foreign = arena.decl_in(NominalContextId::fresh(), "display", 1);
        let generic = arena.generic_type(arena.intern_generic(GenericParamData {
            name: "T".into(),
            kind: Default::default(),
            owner_symbol_index: 0,
            bound: None,
        }));
        for value in [foreign, generic] {
            let key = arena.intern(Type::Literal(LitValue::Str("a".into())));
            let ty = arena.intern(Type::Operator(TypeOperator::Object(vec![TypeProperty {
                key,
                value,
                optional: false,
                readonly: false,
                index: false,
            }])));
            assert_eq!(relation.canonical(ty, 0), None);
        }
    });
}
