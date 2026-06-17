use super::kind_ok_table_for_test;
use crate::languages::javascript::profile::JAVASCRIPT_PROFILE;
use crate::languages::typescript::profile::TYPESCRIPT_PROFILE;
use crate::types::EdgeKind;

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
