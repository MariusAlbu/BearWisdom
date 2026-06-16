use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn apply(lookup: &Lookup, target: &str, imports: Vec<ImportEntry>) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match AliasModuleQnameRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

fn apply_with_gate(
    lookup: &Lookup,
    target: &str,
    imports: Vec<ImportEntry>,
) -> Option<i64> {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            alias_module_qname: true,
            ..DEFAULT_PROFILE
        };
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &PROFILE,
    };
    match AliasModuleQnameRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Gate is `false` by default — rule passes without checking imports.
#[test]
fn passes_when_gate_off() {
    let lookup = Lookup::new().with(sym(1, "Foo", "MyApp.Foo", "module", "src/foo.dart"));
    let imports = vec![import("Foo", Some("MyApp.Foo"))];
    assert_eq!(apply(&lookup, "Foo", imports), None);
}

/// Gate on: bare import name matched against the import's module_path as a qname.
#[test]
fn binds_bare_import_to_module_qname() {
    let lookup = Lookup::new().with(sym(42, "Foo", "MyApp.Foo", "module", "src/foo.dart"));
    let imports = vec![import("Foo", Some("MyApp.Foo"))];
    assert_eq!(apply_with_gate(&lookup, "Foo", imports), Some(42));
}

/// Dotted target is rejected — this rule only handles bare names.
#[test]
fn declines_dotted_target() {
    let lookup = Lookup::new().with(sym(5, "Foo", "MyApp.Foo", "module", "src/foo.dart"));
    let imports = vec![import("Foo", Some("MyApp.Foo"))];
    assert_eq!(apply_with_gate(&lookup, "Foo.Bar", imports), None);
}

/// Import present but no qname match in the index.
#[test]
fn declines_when_qname_not_in_index() {
    let lookup = Lookup::new(); // empty index
    let imports = vec![import("Foo", Some("MyApp.Foo"))];
    assert_eq!(apply_with_gate(&lookup, "Foo", imports), None);
}
