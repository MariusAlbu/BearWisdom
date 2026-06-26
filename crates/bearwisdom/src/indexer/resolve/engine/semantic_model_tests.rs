use super::{chain_root_is_namespace, chain_root_is_wildcard_import, kind_ok_table_for_test};
use crate::indexer::resolve::engine::testkit::{file_ctx, import, sym, Lookup};
use crate::languages::javascript::profile::JAVASCRIPT_PROFILE;
use crate::languages::typescript::profile::TYPESCRIPT_PROFILE;
use crate::types::{ChainSegment, EdgeKind, MemberChain, SegmentKind};

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
