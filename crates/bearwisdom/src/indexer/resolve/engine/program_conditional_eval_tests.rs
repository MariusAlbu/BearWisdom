use super::super::super::tests::with_relation;
use super::*;

#[test]
fn unsupported_or_foreign_pattern_is_not_a_false_condition() {
    with_relation(|relation| {
        let arena = relation.arena;
        let yes = arena.intern(Type::Intrinsic(Intrinsic::String));
        let no = arena.intern(Type::Intrinsic(Intrinsic::Number));
        for pattern in [
            arena.intern(Type::Unknown),
            arena.decl_in(relation.lookup.view.context, "missing", 999),
        ] {
            assert_eq!(
                Eval::new(relation).conditional(yes, pattern, yes, no, Some(false), 0),
                None
            );
        }
        let function = arena.intern(Type::Function {
            params: vec![yes],
            return_: yes,
        });
        assert_eq!(
            Eval::new(relation).conditional(function, no, yes, no, Some(false), 0),
            None
        );
    });
}

#[test]
fn an_unowned_infer_parameter_cannot_prove_a_missing_property_branch() {
    with_relation(|relation| {
        let arena = relation.arena;
        let parameter = arena.intern_generic(crate::type_checker::core::types::GenericParamData {
            name: "display".into(),
            owner_symbol_index: 0,
            kind: crate::type_checker::core::types::GenericParamKind::Type,
            bound: None,
        });
        let pattern = arena.intern(Type::Operator(TypeOperator::Object(vec![TypeProperty {
            key: arena.intern(Type::Literal(LitValue::Str("value".into()))),
            value: arena.intern(Type::Operator(TypeOperator::Infer(
                arena.generic_type(parameter),
            ))),
            optional: false,
            readonly: false,
            index: false,
        }])));
        let check = arena.intern(Type::Operator(TypeOperator::Object(vec![])));
        assert_eq!(
            Eval::new(relation).conditional(check, pattern, check, check, Some(false), 0),
            None
        );
    });
}

#[test]
fn alternate_literal_encodings_do_not_become_negative_equality_evidence() {
    with_relation(|relation| {
        let arena = relation.arena;
        let yes = arena.intern(Type::Intrinsic(Intrinsic::String));
        let no = arena.intern(Type::Intrinsic(Intrinsic::Number));
        for (left, right) in [
            (LitValue::Int(1), LitValue::Number(1f64.to_bits())),
            (LitValue::Str("x".into()), LitValue::Utf16(vec![120])),
        ] {
            let left = arena.intern(Type::Literal(left));
            let right = arena.intern(Type::Literal(right));
            assert_eq!(
                Eval::new(relation).conditional(left, right, yes, no, Some(false), 0),
                None
            );
        }
    });
}
