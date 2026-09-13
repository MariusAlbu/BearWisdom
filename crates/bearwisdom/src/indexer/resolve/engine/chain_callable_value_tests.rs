use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};

/// A package whose `CheckStatic` declaration carries a call signature returning
/// `Check`, plus the property `soft` typed by that same declaration.
fn checks() -> Lookup {
    Lookup::new()
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
        .with_return_type("CheckStatic.call", "Check")
        .with_member(
            "CheckStatic",
            sym(
                51,
                "soft",
                "CheckStatic.soft",
                "property",
                "ext:ts:checks/index.d.ts",
            ),
        )
        .with_field_type("CheckStatic.soft", "CheckStatic")
        .with(sym(
            3,
            "Check",
            "Check",
            "interface",
            "ext:ts:checks/index.d.ts",
        ))
}

#[test]
fn an_inline_signature_yields_its_return_and_names_no_declaration() {
    let arena = TypeArena::new();
    let return_ = arena.class("Client");
    let ty = arena.intern(Type::Function {
        params: vec![],
        return_,
    });

    let yielded = call_yield(&Lookup::new(), &arena, ty).unwrap();

    assert_eq!(yielded.ty, return_);
    assert!(
        yielded.signature_id.is_none(),
        "an inline signature has no declaration of its own"
    );
}

#[test]
fn a_declarations_call_signature_yields_its_return_and_names_the_signature() {
    let arena = TypeArena::new();

    let yielded = call_yield(&checks(), &arena, arena.class("CheckStatic")).unwrap();

    assert_eq!(head_qname(&arena, yielded.ty).as_deref(), Some("Check"));
    assert_eq!(yielded.signature_id, Some(50));
}

#[test]
fn a_declaration_without_a_call_signature_is_not_callable() {
    let arena = TypeArena::new();
    let lookup = Lookup::new()
        .with(sym(2, "Check", "Check", "interface", "a.ts"))
        .with_member("Check", sym(40, "toBe", "Check.toBe", "method", "a.ts"));

    assert!(call_yield(&lookup, &arena, arena.class("Check")).is_none());
}

#[test]
fn calling_a_property_typed_by_a_callable_declaration_yields_the_signatures_return() {
    let arena = TypeArena::new();
    let soft = sym(
        51,
        "soft",
        "CheckStatic.soft",
        "property",
        "ext:ts:checks/index.d.ts",
    );

    let yielded = member_yield_type(&checks(), &arena, &soft, true).unwrap();

    assert_eq!(
        head_qname(&arena, yielded).as_deref(),
        Some("Check"),
        "calling the property calls the value it holds, not the type describing it"
    );
}

#[test]
fn reading_a_property_typed_by_a_callable_declaration_keeps_the_declaration() {
    let arena = TypeArena::new();
    let soft = sym(
        51,
        "soft",
        "CheckStatic.soft",
        "property",
        "ext:ts:checks/index.d.ts",
    );

    let read = member_yield_type(&checks(), &arena, &soft, false).unwrap();

    assert_eq!(head_qname(&arena, read).as_deref(), Some("CheckStatic"));
}

#[test]
fn a_self_referential_call_signature_does_not_re_enter_its_own_probe() {
    let arena = TypeArena::new();
    let signature = sym(50, "call", "Loop.call", "property", "a.ts");
    let lookup = Lookup::new()
        .with(sym(2, "Loop", "Loop", "interface", "a.ts"))
        .with_member("Loop", signature.clone())
        .with_field_type("Loop.call", "Loop");

    let yielded = member_yield_type(&lookup, &arena, &signature, true).unwrap();

    assert_eq!(head_qname(&arena, yielded).as_deref(), Some("Loop"));
}
