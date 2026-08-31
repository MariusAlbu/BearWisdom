use super::{
    chain_root_is_namespace, chain_root_is_wildcard_import, kind_ok_table_for_test, SemanticModel,
    SolveOutcome,
};
use crate::indexer::resolve::engine::testkit::{file_ctx, import, ref_ctx, source_symbol, sym, Lookup};
use crate::languages::javascript::profile::JAVASCRIPT_PROFILE;
use crate::languages::rust_lang::profile::RUST_PROFILE;
use crate::languages::typescript::profile::TYPESCRIPT_PROFILE;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};

fn nseg(name: &str) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind: SegmentKind::NamespaceAccess,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

/// `React.useState` — the root names a module/namespace, so a chain the walker
/// declined falls through to the bare-name ladder (which binds the member under
/// the imported namespace) instead of being a hard miss.
#[test]
fn namespace_rooted_chain_is_detected() {
    let lookup =
        Lookup::new().with(sym(1, "React", "@types/react.React", "module", "react/index.d.ts"));
    let chain = MemberChain {
        segments: vec![nseg("React"), nseg("useState")],
    };
    assert!(chain_root_is_namespace(&chain, &lookup));
}

/// A qualified call chain the walker can't root (`m::f(v)` — the root names a
/// module path, never a typable value) but whose ref carries extractor-set
/// `module` evidence falls through to the ladder, where the module anchor
/// binds the target inside the module's own files.
#[test]
fn module_tagged_declined_chain_falls_through_to_ladder() {
    let lookup = Lookup::new().with(sym(
        7,
        "from_value",
        "from_value",
        "function",
        "ext:rust:ser_x/src/value/mod.rs",
    ));
    let seg = |name: &str, kind: SegmentKind, is_call: bool| ChainSegment {
        name: name.to_string(),
        node_kind: "scoped_identifier".to_string(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    };
    let mut r = ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "from_value".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: Some("ser_x".to_string()),
        namespace_segments: Vec::new(),
        chain: Some(MemberChain {
            segments: vec![
                seg("ser_x", SegmentKind::Identifier, false),
                seg("from_value", SegmentKind::Property, true),
            ],
        }),
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let src = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let solver = SemanticModel::production();

    let rc = ref_ctx(&r, &src, vec![]);
    match solver.get_symbol_info(&rc, &fc, &lookup, &RUST_PROFILE) {
        SolveOutcome::Resolved(res) => assert_eq!(res.target_symbol_id, 7),
        _ => panic!("module-tagged declined chain must reach the module anchor"),
    }

    // Control: the same declined chain WITHOUT module evidence stays a hard
    // miss — no ladder fall-through, no sibling hijack.
    r.module = None;
    let rc = ref_ctx(&r, &src, vec![]);
    assert!(matches!(
        solver.get_symbol_info(&rc, &fc, &lookup, &RUST_PROFILE),
        SolveOutcome::Unresolved(_)
    ));

    // Control: a module tag that is NOT the chain's own qualifier (it names
    // where the ROOT was imported from) must not fall through either — even
    // though the module-scoped rungs could locate the same target under it.
    r.module = Some("other_pkg".to_string());
    let rc = ref_ctx(&r, &src, vec![]);
    assert!(matches!(
        solver.get_symbol_info(&rc, &fc, &lookup, &RUST_PROFILE),
        SolveOutcome::Unresolved(_)
    ));
}

/// A module-tagged declined chain runs ONLY the module-evidence rungs: when
/// the module locates nothing, a same-named candidate that a full-ladder rung
/// (same-file / ambient / global) would bind must stay untouched — the module
/// evidence scopes the fall-through, it does not widen it.
#[test]
fn module_tagged_declined_chain_cannot_hijack_a_same_named_sibling() {
    // The only `parse` in the index lives in the REF'S OWN FILE — the classic
    // same-file hijack bait (the fixture file context is `src/main.ts`). The
    // ref's module names a package that declares nothing, so the
    // module-scoped rungs all miss.
    let lookup = Lookup::new().with(sym(9, "parse", "parse", "function", "src/main.ts"));
    let seg = |name: &str, kind: SegmentKind, is_call: bool| ChainSegment {
        name: name.to_string(),
        node_kind: "scoped_identifier".to_string(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    };
    let r = ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "parse".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: Some("other_crate".to_string()),
        namespace_segments: Vec::new(),
        chain: Some(MemberChain {
            segments: vec![
                seg("other_crate", SegmentKind::Identifier, false),
                seg("parse", SegmentKind::Property, true),
            ],
        }),
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let src = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let solver = SemanticModel::production();
    let rc = ref_ctx(&r, &src, vec![]);
    assert!(
        matches!(
            solver.get_symbol_info(&rc, &fc, &lookup, &RUST_PROFILE),
            SolveOutcome::Unresolved(_)
        ),
        "a module miss must stay a miss — the same-file sibling is not other_crate's parse"
    );
}

/// `rendered.getByText` — `rendered` is a value, not a namespace; the chain
/// walker owns it and a decline stays a hard miss so no same-named sibling
/// hijacks the member access.
#[test]
fn value_rooted_chain_is_not_a_namespace() {
    let lookup = Lookup::new().with(sym(1, "rendered", "rendered", "variable", "x.ts"));
    let chain = MemberChain {
        segments: vec![nseg("rendered"), nseg("getByText")],
    };
    assert!(!chain_root_is_namespace(&chain, &lookup));
}

/// `import * as v from 'valibot'; v.object(...)` — the wildcard alias `v` names
/// the module, not a value. A declined chain falls through to the bare-name
/// ladder, which resolves `object` as a valibot export; without this `v` is
/// value-typed to a foreign same-name binding and the member is a hard miss.
#[test]
fn wildcard_import_rooted_chain_is_detected() {
    let mut imp = import("*", Some("valibot"));
    imp.alias = Some("v".to_string());
    imp.is_wildcard = true;
    let fc = file_ctx(vec![imp], None);
    let chain = MemberChain {
        segments: vec![nseg("v"), nseg("object")],
    };
    assert!(chain_root_is_wildcard_import(&chain, &fc));
}

/// A named (non-wildcard) import is a value the chain walker owns — a decline
/// stays a hard miss, no ladder fall-through.
#[test]
fn named_import_rooted_chain_is_not_wildcard() {
    let fc = file_ctx(vec![import("foo", Some("m"))], None);
    let chain = MemberChain {
        segments: vec![nseg("foo"), nseg("bar")],
    };
    assert!(!chain_root_is_wildcard_import(&chain, &fc));
}

/// The extractor emits `namespace X {}` / `declare namespace X` as a `Module`
/// (and a bare `namespace`-kind for some shapes). A namespace value root —
/// `Reflect.set`, `React.FC` — reaches the binder as a `TypeRef`, so the TS
/// profile's kind table must admit both `module` and `namespace` for a
/// `TypeRef` edge, or the root never binds. This is the gate the rule ladder
/// consults via `BinderContext.kind` (built from `profile.kind_compatible_table`).
#[test]
fn ts_typeref_admits_module_and_namespace_kinds() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    assert!(
        kind_ok_table_for_test(table, EdgeKind::TypeRef, "module"),
        "TS TypeRef must admit a `module`-kind namespace declaration"
    );
    assert!(
        kind_ok_table_for_test(table, EdgeKind::TypeRef, "namespace"),
        "TS TypeRef must admit a `namespace`-kind declaration"
    );
}

/// The JS profile admits a `module`-kind namespace root for a `TypeRef`
/// (`Reflect.set`). JS has no `namespace` keyword, so the extractor emits
/// namespaces as `Module` only — the table carries `module`, not `namespace`.
#[test]
fn js_typeref_admits_module_kind() {
    let table = JAVASCRIPT_PROFILE.kind_compatible_table;
    assert!(
        kind_ok_table_for_test(table, EdgeKind::TypeRef, "module"),
        "JS TypeRef must admit a `module`-kind namespace declaration"
    );
}

/// An unrecognised symbol-kind string defaults permissive — an extractor typo
/// must not silently hide a real symbol — so the table is not a closed allowlist.
#[test]
fn unknown_kind_defaults_permissive() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    assert!(
        kind_ok_table_for_test(table, EdgeKind::TypeRef, "not_a_real_kind"),
        "an unparseable kind defaults permissive"
    );
}
