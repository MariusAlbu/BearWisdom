use super::*;
use crate::indexer::resolve::engine::testkit::{file_ctx, sym, Lookup};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

/// A root segment; the name never carries identity here — the bound
/// `LocalReference` does.
fn root_segment(is_call: bool) -> ChainSegment {
    ChainSegment {
        name: "not-used-for-identity".into(),
        node_kind: String::new(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: vec![],
        optional_chaining: false,
        byte_offset: 10,
        declared_type_id: None,
        is_call,
        call_args: vec![],
        type_arg_ids: vec![],
    }
}

/// A binding whose declaration is `declaration` and which carries no type of
/// its own — the shape an import binding of an ambient value takes.
fn untyped_binding(declaration: i64) -> LocalReference {
    LocalReference {
        declaration: Some(declaration),
        kind: SymbolKind::Variable,
        value_type: None,
        callable: None,
        type_args: vec![],
    }
}

#[test]
fn callable_yield_keeps_a_canonical_return_id_without_nominalizing_it() {
    let arena = TypeArena::new();
    let expected = arena.intern(Type::Optional(arena.decl("SameDisplay", 71)));
    let ty = arena.intern(Type::Function {
        params: vec![],
        return_: expected,
    });
    let local = LocalReference {
        declaration: Some(3),
        kind: SymbolKind::Parameter,
        value_type: Some(ty),
        callable: None,
        type_args: vec![],
    };
    let result = resolve(
        local,
        &Lookup::new(),
        &arena,
        &file_ctx(vec![], None),
        &root_segment(true),
        &DEFAULT_PROFILE,
    )
    .unwrap();
    assert_eq!(result.ty, expected);
}

#[test]
fn a_bound_value_whose_declared_type_is_callable_yields_its_call_signature() {
    let arena = TypeArena::new();
    let lookup = Lookup::new()
        .with(sym(
            1,
            "check",
            "check",
            "variable",
            "ext:ts:checks/index.d.ts",
        ))
        .with_field_type("check", "CheckStatic")
        .with(sym(
            2,
            "CheckStatic",
            "CheckStatic",
            "interface",
            "ext:ts:checks/index.d.ts",
        ))
        .with_member(
            "CheckStatic",
            sym(
                50,
                "call",
                "CheckStatic.call",
                "method",
                "ext:ts:checks/index.d.ts",
            ),
        )
        .with_return_type("CheckStatic.call", "Check");

    let result = resolve(
        untyped_binding(1),
        &lookup,
        &arena,
        &file_ctx(vec![], None),
        &root_segment(true),
        &DEFAULT_PROFILE,
    )
    .unwrap();

    assert_eq!(head_qname(&arena, result.ty).as_deref(), Some("Check"));
}

#[test]
fn a_bound_value_with_no_callable_type_still_blames_its_declaration() {
    let arena = TypeArena::new();
    let lookup = Lookup::new()
        .with(sym(1, "opaque", "opaque", "variable", "a.ts"))
        .with_field_type("opaque", "Bag")
        .with(sym(2, "Bag", "Bag", "interface", "a.ts"));

    let cause = resolve(
        untyped_binding(1),
        &lookup,
        &arena,
        &file_ctx(vec![], None),
        &root_segment(true),
        &DEFAULT_PROFILE,
    )
    .unwrap_err()
    .unwrap();

    assert_eq!(cause.symbol_id, Some(1));
    assert_eq!(cause.kind, CauseKind::UncapturedReturn);
}

#[test]
fn a_bound_value_with_no_type_of_its_own_roots_on_its_declarations_type() {
    let arena = TypeArena::new();
    let lookup = Lookup::new()
        .with(sym(1, "box", "box", "variable", "ext:ts:boxes/index.d.ts"))
        .with_field_type("box", "Holder")
        .with(sym(
            2,
            "Holder",
            "Holder",
            "interface",
            "ext:ts:boxes/index.d.ts",
        ));

    let result = resolve(
        untyped_binding(1),
        &lookup,
        &arena,
        &file_ctx(vec![], None),
        &root_segment(false),
        &DEFAULT_PROFILE,
    )
    .unwrap();

    assert_eq!(head_qname(&arena, result.ty).as_deref(), Some("Holder"));
}
