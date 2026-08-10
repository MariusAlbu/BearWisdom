use super::{bind_explicit_type_args, with_segment_args};
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::Type;
use crate::types::{ChainSegment, SegmentKind};

fn call_seg(name: &str, type_args: &[&str]) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind: SegmentKind::Property,
        declared_type: None,
        type_args: type_args.iter().map(|t| t.to_string()).collect(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call: true,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

/// A single explicit argument binds the callee's single param in the yield.
#[test]
fn one_argument_binds_one_param() {
    let lookup = Lookup::new().with_generics("Sut.GetDependency", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(11, "GetDependency", "Sut.GetDependency", "method", "src/S.cs");
    let yielded = arena.intern_type_str("T");
    let bound = bind_explicit_type_args(
        &lookup,
        arena,
        &member,
        &call_seg("GetDependency", &["IOrgRepo"]),
        yielded,
    );
    assert!(matches!(arena.get(bound), Type::Class(n) if n == "IOrgRepo"));
}

/// Params beyond the supplied arguments stay open — only the covered
/// positions rewrite.
#[test]
fn a_partial_argument_list_binds_a_prefix() {
    let lookup = Lookup::new().with_generics("M.pair", &["A", "B"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(11, "pair", "M.pair", "method", "src/M.ts");
    let yielded = arena.intern_type_str("Map<A, B>");
    let bound = bind_explicit_type_args(&lookup, arena, &member, &call_seg("pair", &["K"]), yielded);
    let formatted = arena.format_type(bound);
    assert!(formatted.contains('K'), "first param bound: {formatted}");
    assert!(formatted.contains('B'), "second param stays open: {formatted}");
}

/// No declared params → the yield passes through untouched.
#[test]
fn a_non_generic_callee_is_untouched() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();
    let member = sym(11, "plain", "M.plain", "method", "src/M.ts");
    let yielded = arena.intern_type_str("Widget");
    assert_eq!(
        bind_explicit_type_args(&lookup, arena, &member, &call_seg("plain", &["X"]), yielded),
        yielded,
    );
}

/// No explicit arguments → untouched, even for a generic callee.
#[test]
fn an_argless_segment_is_untouched() {
    let lookup = Lookup::new().with_generics("M.get", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(11, "get", "M.get", "method", "src/M.ts");
    let yielded = arena.intern_type_str("T");
    assert_eq!(
        bind_explicit_type_args(&lookup, arena, &member, &call_seg("get", &[]), yielded),
        yielded,
    );
}

/// A bare class head gains the segment's args as an application; a head that
/// already carries args is left alone.
#[test]
fn segment_args_attach_only_to_a_bare_head() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();
    let bare = arena.intern_type_str("Repository");
    let attached = with_segment_args(arena, bare, &["User".to_string()]);
    assert!(matches!(arena.get(attached), Type::Apply { .. }));
    let applied = arena.intern_type_str("Repository<Order>");
    assert_eq!(
        with_segment_args(arena, applied, &["User".to_string()]),
        applied,
    );
}
