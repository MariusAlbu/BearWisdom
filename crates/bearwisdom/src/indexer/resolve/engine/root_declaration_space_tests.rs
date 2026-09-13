use super::{is_external_member, value_yields_to_type};
use crate::indexer::resolve::engine::chain::bind_member_access;
use crate::indexer::resolve::engine::contract::{Symbol, SymbolSet};
use crate::indexer::resolve::engine::testkit::{
    call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::types::{ChainSegment, MemberChain, SegmentKind};

/// The value face of a global, as the externals pipeline records it.
const LIB_VALUE_FILE: &str = "ext:ts:__ts_lib__/lib.es2015.promise.d.ts";
/// The shape face of the same global, written in another file of one package.
const LIB_SHAPE_FILE: &str = "ext:ts:__ts_lib__/lib.es2015.iterable.d.ts";

fn set(symbols: &[Symbol]) -> SymbolSet<'_> {
    SymbolSet::Borrowed(symbols)
}

#[test]
fn a_split_globals_shape_face_leaves_the_root_on_its_value() {
    let value = sym(1, "Promise", "Promise", "variable", LIB_VALUE_FILE);
    let shape = [sym(2, "Promise", "Promise", "interface", LIB_SHAPE_FILE)];
    assert!(!value_yields_to_type(&value, &set(&shape)));
}

#[test]
fn a_same_file_pair_leaves_the_root_on_its_value() {
    let value = sym(1, "Date", "Date", "variable", LIB_VALUE_FILE);
    let shape = [sym(2, "Date", "Date", "interface", LIB_VALUE_FILE)];
    assert!(!value_yields_to_type(&value, &set(&shape)));
}

/// The value keeps the root only while every declaration it groups with is a
/// shape. A declaration that evaluates in its own right — a class, a merged
/// namespace — is the static surface the bare name denotes.
#[test]
fn an_evaluable_declaration_in_the_package_takes_the_root_back() {
    let value = sym(1, "Widget", "pkg.Widget", "variable", LIB_VALUE_FILE);
    let evaluable = [sym(2, "Widget", "pkg.Widget", "class", LIB_SHAPE_FILE)];
    assert!(value_yields_to_type(&value, &set(&evaluable)));
}

#[test]
fn a_same_named_value_in_another_package_stays_a_stranger() {
    let value = sym(1, "P", "otherpkg.P", "variable", "ext:ts:otherpkg/index.d.ts");
    let shape = [sym(2, "P", "P", "interface", "ext:ts:somepkg/index.d.ts")];
    assert!(value_yields_to_type(&value, &set(&shape)));
}

/// Package grammar is ecosystem-owned. A virtual path no adapter claims proves
/// no scope wider than its own file, so a cross-file pair stays two names.
#[test]
fn a_path_with_no_package_grammar_proves_nothing_beyond_its_file() {
    let value = sym(1, "P", "otherpkg.P", "variable", "ext:rust:otherpkg/lib.rs");
    let shape = [sym(2, "P", "P", "interface", "ext:rust:std/path.rs")];
    assert!(value_yields_to_type(&value, &set(&shape)));
}

#[test]
fn the_projects_own_type_declaration_always_takes_the_root() {
    let value = sym(1, "Config", "Config", "variable", LIB_VALUE_FILE);
    let internal = [sym(2, "Config", "Config", "interface", "src/config.ts")];
    assert!(value_yields_to_type(&value, &set(&internal)));
}

#[test]
fn an_external_member_never_roots_a_bare_name() {
    let member = sym(1, "Promise", "Holder.Promise", "property", LIB_VALUE_FILE);
    let shape = [sym(2, "Promise", "Promise", "interface", LIB_SHAPE_FILE)];
    assert!(is_external_member(&member));
    assert!(value_yields_to_type(&member, &set(&shape)));
}

#[test]
fn an_internal_value_is_never_subject_to_the_external_yield() {
    let value = sym(1, "Promise", "Promise", "variable", "src/shim.ts");
    let shape = [sym(2, "Promise", "Promise", "interface", LIB_SHAPE_FILE)];
    assert!(!is_external_member(&value));
    assert!(!value_yields_to_type(&value, &set(&shape)));
}

#[test]
fn a_value_keeps_the_root_when_the_name_declares_no_type_at_all() {
    let value = sym(1, "clone", "clone", "variable", LIB_VALUE_FILE);
    assert!(!value_yields_to_type(&value, &set(&[])));
}

fn seg(name: &str, is_call: bool, kind: SegmentKind) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

fn resolve(lookup: &Lookup, segs: Vec<ChainSegment>) -> Option<i64> {
    let leaf = segs.last().unwrap().name.clone();
    let mut r = call_ref(&leaf);
    r.chain = Some(MemberChain { segments: segs });
    let mut s = source_symbol("caller");
    s.qualified_name = "caller".to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(
        &rc,
        &file_ctx(vec![], None),
        lookup,
        &crate::languages::typescript::TYPESCRIPT_PROFILE,
    )
    .ok()
    .map(|res| res.target_symbol_id)
}

/// The whole chain: `Promise.resolve()` on a global whose constructor value and
/// instance shape are declared in different files of one package. The static
/// surface is on the value's declared type, so the walk must find
/// `PromiseConstructor.resolve` rather than miss on the instance shape.
#[test]
fn a_split_globals_static_surface_resolves_through_its_value() {
    let lookup = Lookup::new()
        .with(sym(1, "Promise", "Promise", "variable", LIB_VALUE_FILE))
        .with_field_type("Promise", "PromiseConstructor")
        .with(sym(2, "Promise", "Promise", "interface", LIB_SHAPE_FILE))
        .with_member_id(2, sym(3, "then", "Promise.then", "method", LIB_SHAPE_FILE))
        .with(sym(
            4,
            "PromiseConstructor",
            "PromiseConstructor",
            "interface",
            LIB_SHAPE_FILE,
        ))
        .with_member_id(
            4,
            sym(
                5,
                "resolve",
                "PromiseConstructor.resolve",
                "method",
                LIB_SHAPE_FILE,
            ),
        );
    let segs = vec![
        seg("Promise", false, SegmentKind::Identifier),
        seg("resolve", true, SegmentKind::Property),
    ];
    assert_eq!(
        resolve(&lookup, segs),
        Some(5),
        "the constructor object's static surface carries `resolve`"
    );
}
