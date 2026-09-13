use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::types::AliasTarget;

/// Depth budget matching the one the chain walker enters a member step with.
const DEPTH: usize = 6;

/// `interface Widget { spin() }` plus whatever else the caller registers.
fn widget() -> Lookup {
    Lookup::new()
        .with(sym(3, "Widget", "Widget", "interface", "pkg/w.ts"))
        .with_member("Widget", sym(99, "spin", "Widget.spin", "method", "pkg/w.ts"))
}

fn union_member(lookup: &Lookup, head: &str, member: &str) -> Option<Symbol> {
    let arena = lookup.type_arena().unwrap();
    lookup_member_on_union(
        lookup,
        arena,
        arena.class(head),
        head,
        member,
        &|_kind| true,
        DEPTH,
    )
}

/// The arm walk's answer for `member` on `recv_ty`, asserting the receiver was
/// recognized as composite.
fn arm_member(lookup: &Lookup, recv_ty: TypeId, member: &str) -> Option<Symbol> {
    let arena = lookup.type_arena().unwrap();
    match lookup_member_on_arms(
        lookup,
        arena,
        Receiver::untyped(recv_ty),
        member,
        &|_kind| true,
        DEPTH,
    ) {
        CompositeLookup::Answered(found) => found,
        CompositeLookup::NotComposite => panic!("a composite type answers through its arms"),
    }
}

#[test]
fn union_alias_member_resolves_past_an_absent_arm() {
    // type Maybe = Widget | null — the absent arm carries no member surface, so
    // it does not decide whether the union carries `spin`.
    let lookup = widget()
        .with(sym(2, "Maybe", "Maybe", "type_alias", "pkg/w.ts"))
        .with_alias(
            "Maybe",
            AliasTarget::Union(vec!["Widget".into(), "null".into()]),
        );
    assert_eq!(union_member(&lookup, "Maybe", "spin").map(|m| m.id), Some(99));
}

#[test]
fn union_alias_member_resolves_past_an_undefined_arm() {
    // type Maybe = Widget | undefined — the other absence spelling behaves the
    // same, because both intern as the same kind of semantic atom.
    let lookup = widget()
        .with(sym(2, "Maybe", "Maybe", "type_alias", "pkg/w.ts"))
        .with_alias(
            "Maybe",
            AliasTarget::Union(vec!["Widget".into(), "undefined".into()]),
        );
    assert_eq!(union_member(&lookup, "Maybe", "spin").map(|m| m.id), Some(99));
}

#[test]
fn union_alias_of_only_absent_arms_carries_nothing() {
    // type Nothing = null | undefined — no arm participates, so no member exists.
    let lookup = widget()
        .with(sym(2, "Nothing", "Nothing", "type_alias", "pkg/w.ts"))
        .with_alias(
            "Nothing",
            AliasTarget::Union(vec!["null".into(), "undefined".into()]),
        );
    assert!(union_member(&lookup, "Nothing", "spin").is_none());
}

#[test]
fn union_alias_member_still_declines_when_a_present_arm_lacks_it() {
    // type Mixed = Widget | Gadget — `Gadget` carries a member surface and does
    // not declare `spin`, so the access is invalid on the union.
    let lookup = widget()
        .with(sym(2, "Mixed", "Mixed", "type_alias", "pkg/w.ts"))
        .with(sym(4, "Gadget", "Gadget", "interface", "pkg/w.ts"))
        .with_alias(
            "Mixed",
            AliasTarget::Union(vec!["Widget".into(), "Gadget".into()]),
        );
    assert!(union_member(&lookup, "Mixed", "spin").is_none());
}

#[test]
fn structural_union_member_resolves_past_an_absent_arm() {
    // A declared `Widget | null` reaches the walk as a union TYPE with no
    // nominal head; the same participation rule applies to its arms.
    let lookup = widget();
    let arena = lookup.type_arena().unwrap();
    let recv_ty = arena.intern(Type::Union(vec![
        arena.class("Widget"),
        arena.intern(Type::Intrinsic(Intrinsic::Null)),
    ]));
    assert_eq!(arm_member(&lookup, recv_ty, "spin").map(|m| m.id), Some(99));
}

#[test]
fn structural_intersection_member_resolves_on_the_arm_that_declares_it() {
    let lookup = widget().with(sym(4, "Gadget", "Gadget", "interface", "pkg/w.ts"));
    let arena = lookup.type_arena().unwrap();
    let recv_ty = arena.intern(Type::Intersection(vec![
        arena.class("Gadget"),
        arena.class("Widget"),
    ]));
    assert_eq!(arm_member(&lookup, recv_ty, "spin").map(|m| m.id), Some(99));
}

#[test]
fn a_nominal_receiver_is_not_composite() {
    // The head-keyed walks own a plain nominal; the arm walk must stand aside
    // rather than answer `None` for it.
    let lookup = widget();
    let arena = lookup.type_arena().unwrap();
    let recv = Receiver::untyped(arena.class("Widget"));
    assert!(matches!(
        lookup_member_on_arms(&lookup, arena, recv, "spin", &|_kind| true, DEPTH),
        CompositeLookup::NotComposite
    ));
}
