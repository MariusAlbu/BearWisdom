use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    DEFAULT_PROFILE, LanguageProfile, WildcardMatch,
};

static FILESTEM_PROFILE: LanguageProfile = LanguageProfile {
    wildcard_match: WildcardMatch::FileStem {
        underscore_prefix: false,
    },
    ..DEFAULT_PROFILE
};

fn wildcard_import(name: &str, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

fn resolve(lookup: &Lookup, target: &str, imports: Vec<ImportEntry>) -> Option<i64> {
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
    match WildcardImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_direct_qname_member_under_wildcard() {
    // `use my::mod::*` brings `my::mod::Foo` into scope; target `Foo` resolves.
    let lookup = Lookup::new().with(sym(10, "Foo", "my.mod.Foo", "class", "src/mod.rs"));
    let imports = vec![wildcard_import("*", "my.mod")];
    assert_eq!(resolve(&lookup, "Foo", imports), Some(10));
}

#[test]
fn declines_when_no_wildcard_imports_present() {
    let lookup = Lookup::new().with(sym(10, "Foo", "my.mod.Foo", "class", "src/mod.rs"));
    // A non-wildcard import — WildcardImportRule must decline.
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("my.mod".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

#[test]
fn declines_dotted_target() {
    let lookup = Lookup::new().with(sym(10, "Foo", "my.mod.Foo", "class", "src/mod.rs"));
    let imports = vec![wildcard_import("*", "my.mod")];
    // Dotted targets are qualified — this rule declines them.
    assert_eq!(resolve(&lookup, "my.mod.Foo", imports), None);
}

#[test]
fn declines_when_symbol_is_not_direct_member() {
    // `my.mod.sub.Foo` is two segments deeper — NOT a direct member of `my.mod`.
    let lookup =
        Lookup::new().with(sym(10, "Foo", "my.mod.sub.Foo", "class", "src/mod.rs"));
    let imports = vec![wildcard_import("*", "my.mod")];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

#[test]
fn filestem_mode_binds_by_file_basename() {
    // WildcardMatch::FileStem — the symbol's file stem must match the module name.
    let lookup = Lookup::new().with(sym(20, "Bar", "Bar", "class", "src/utils.rs"));
    let imports = vec![wildcard_import("*", "utils")];
    let r = call_ref("Bar");
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &FILESTEM_PROFILE,
    };
    match WildcardImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 20),
        _ => panic!("expected Resolved"),
    }
}
