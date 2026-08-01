use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;
use crate::type_checker::core::types::Type;

#[test]
fn a_string_literal_argument_types_as_the_string_primitive() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(&lookup, arena, &[CallArg::StringLit("users".into())]);

    assert_eq!(arena.get(tys[0]), Type::Primitive(PrimKind::Str));
}

#[test]
fn numeric_and_boolean_literals_type_as_their_primitives() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(
        &lookup,
        arena,
        &[
            CallArg::Literal("42".into()),
            CallArg::Literal("1.5".into()),
            CallArg::Literal("true".into()),
        ],
    );

    assert_eq!(arena.get(tys[0]), Type::Primitive(PrimKind::Int));
    assert_eq!(arena.get(tys[1]), Type::Primitive(PrimKind::Float));
    assert_eq!(arena.get(tys[2]), Type::Primitive(PrimKind::Bool));
}

#[test]
fn a_null_literal_declines() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(&lookup, arena, &[CallArg::Literal("null".into())]);

    assert_eq!(arena.get(tys[0]), Type::Unknown);
}

#[test]
fn an_identifier_argument_takes_its_forward_inferred_local_type() {
    let lookup = Lookup::new().with_local_type("user", "User");
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(&lookup, arena, &[CallArg::Ident("user".into())]);

    assert_eq!(arena.get(tys[0]), Type::Class("User".to_string()));
}

#[test]
fn an_unknown_identifier_declines() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(&lookup, arena, &[CallArg::Ident("mystery".into())]);

    assert_eq!(arena.get(tys[0]), Type::Unknown);
}

#[test]
fn a_homogeneous_array_literal_types_as_the_sequence_of_its_element() {
    let lookup = Lookup::new().with_local_type("user", "User");
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(
        &lookup,
        arena,
        &[CallArg::ArrayLiteral {
            elements: vec![CallArg::Ident("user".into()), CallArg::Ident("user".into())],
        }],
    );

    let Type::Apply { base, args } = arena.get(tys[0]) else {
        panic!("expected an Apply, got {:?}", arena.get(tys[0]));
    };
    assert_eq!(arena.get(base), Type::Class("Array".to_string()));
    assert_eq!(arena.get(args[0]), Type::Class("User".to_string()));
}

#[test]
fn a_mixed_array_literal_declines() {
    let lookup = Lookup::new().with_local_type("user", "User");
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(
        &lookup,
        arena,
        &[CallArg::ArrayLiteral {
            elements: vec![
                CallArg::Ident("user".into()),
                CallArg::StringLit("x".into()),
            ],
        }],
    );

    assert_eq!(arena.get(tys[0]), Type::Unknown);
}

#[test]
fn an_awaited_argument_with_no_async_layer_keeps_its_type() {
    let lookup = Lookup::new().with_local_type("user", "User");
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(
        &lookup,
        arena,
        &[CallArg::Await {
            expr: Box::new(CallArg::Ident("user".into())),
        }],
    );

    assert_eq!(arena.get(tys[0]), Type::Class("User".to_string()));
}

#[test]
fn a_ternary_types_only_when_both_branches_agree() {
    let lookup = Lookup::new().with_local_type("user", "User");
    let arena = lookup.type_arena().unwrap();

    let same = resolve_arg_types(
        &lookup,
        arena,
        &[CallArg::Ternary {
            then_branch: Box::new(CallArg::Ident("user".into())),
            else_branch: Box::new(CallArg::Ident("user".into())),
        }],
    );
    let mixed = resolve_arg_types(
        &lookup,
        arena,
        &[CallArg::Ternary {
            then_branch: Box::new(CallArg::Ident("user".into())),
            else_branch: Box::new(CallArg::StringLit("x".into())),
        }],
    );

    assert_eq!(arena.get(same[0]), Type::Class("User".to_string()));
    assert_eq!(arena.get(mixed[0]), Type::Unknown);
}

#[test]
fn positions_stay_aligned_when_one_argument_declines() {
    let lookup = Lookup::new().with_local_type("user", "User");
    let arena = lookup.type_arena().unwrap();

    let tys = resolve_arg_types(
        &lookup,
        arena,
        &[
            CallArg::Other,
            CallArg::Ident("user".into()),
        ],
    );

    assert_eq!(tys.len(), 2);
    assert_eq!(arena.get(tys[0]), Type::Unknown);
    assert_eq!(arena.get(tys[1]), Type::Class("User".to_string()));
}
