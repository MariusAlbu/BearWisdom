use super::super::super::tests::with_relation;
use super::*;

#[test]
fn readonly_tuple_normalization_preserves_optional_slot_arity() {
    with_relation(|relation| {
        let arena = relation.arena;
        let string = arena.intern(Type::Intrinsic(Intrinsic::String));
        let undefined = arena.intern(Type::Intrinsic(Intrinsic::Undefined));
        let optional = arena.intern(Type::Optional(string));
        let required = arena.intern(Type::Union(vec![string, undefined]));
        let tuple = |value| {
            arena.intern(Type::Operator(TypeOperator::Readonly(
                arena.intern(Type::Tuple(vec![value])),
            )))
        };
        let optional = relation.canonical(tuple(optional), 0).unwrap();
        let required = relation.canonical(tuple(required), 0).unwrap();
        assert_ne!(optional, required);
        assert_eq!(relation.canonical(optional, 0), Some(optional));
        assert_ne!(optional, arena.intern(Type::Tuple(vec![string])));
    });
}

#[test]
fn duplicate_or_invalid_object_keys_cannot_prove_identity() {
    with_relation(|relation| {
        let arena = relation.arena;
        let string = arena.intern(Type::Intrinsic(Intrinsic::String));
        let key = arena.intern(Type::Literal(LitValue::Str("a".into())));
        let property = TypeProperty {
            key,
            value: string,
            optional: false,
            readonly: false,
            index: false,
        };
        for properties in [
            vec![property.clone(), property.clone()],
            vec![TypeProperty {
                key: string,
                ..property
            }],
        ] {
            let ty = arena.intern(Type::Operator(TypeOperator::Object(properties)));
            assert!(!relation.equal(ty, ty));
        }
    });
}

#[test]
fn modifiers_remain_distinct_in_equality_but_readonly_does_not_forbid_assignment() {
    with_relation(|relation| {
        let arena = relation.arena;
        let value = arena.intern(Type::Intrinsic(Intrinsic::String));
        let key = arena.intern(Type::Literal(LitValue::Str("a".into())));
        let object = |optional, readonly| {
            arena.intern(Type::Operator(TypeOperator::Object(vec![TypeProperty {
                key,
                value,
                optional,
                readonly,
                index: false,
            }])))
        };
        assert!(!relation.equal(object(false, false), object(false, true)));
        assert!(relation.assignable(object(false, true), object(false, false)));
        assert!(!relation.assignable(object(true, false), object(false, false)));
    });
}
