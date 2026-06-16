use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

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
    match ReexportChainRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Relative import specifiers are skipped — `reexport_following` handles those.
#[test]
fn declines_relative_import() {
    let lookup = Lookup::new().with(sym(1, "Foo", "Foo", "class", "src/foo.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("./foo".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

/// No import matches the target — rule declines.
#[test]
fn declines_when_no_import_matches_target() {
    let lookup = Lookup::new().with(sym(2, "Bar", "Bar", "class", "src/bar.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Other".to_string(),
        module_path: Some("pkg-a".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Bar", imports), None);
}

/// Bare package import with `resolve_external_reexport` returning None (testkit
/// default) — rule declines even when the import name matches.
#[test]
fn declines_when_no_external_reexport_found() {
    let lookup = Lookup::new().with(sym(3, "Foo", "Foo", "class", "src/foo.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("pkg-a".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    // Testkit Lookup::resolve_external_reexport returns None by default.
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

/// Dotted target with no matching import prefix — rule declines.
#[test]
fn declines_dotted_target_no_prefix_import() {
    let lookup = Lookup::new().with(sym(4, "Inner", "Inner", "class", "src/inner.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Other".to_string(),
        module_path: Some("pkg-a".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Ns.Inner", imports), None);
}
