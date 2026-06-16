use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
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
    match NamespaceImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_type_under_dotted_module_namespace() {
    // `using eShop.Catalog.API.Model;` → forms `eShop.Catalog.API.Model.CatalogItem`
    let lookup = Lookup::new().with(sym(
        30,
        "CatalogItem",
        "eShop.Catalog.API.Model.CatalogItem",
        "class",
        "src/model.cs",
    ));
    let imports = vec![import("*", Some("eShop.Catalog.API.Model"))];
    assert_eq!(resolve(&lookup, "CatalogItem", imports), Some(30));
}

#[test]
fn binds_type_under_imported_name_as_namespace() {
    // Import entry with no module_path; `imported_name` is the dotted namespace.
    let lookup = Lookup::new().with(sym(
        31,
        "Helper",
        "App.Utils.Helper",
        "class",
        "src/utils.cs",
    ));
    let imports = vec![ImportEntry {
        imported_name: "App.Utils".to_string(),
        module_path: None,
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Helper", imports), Some(31));
}

#[test]
fn declines_non_dotted_prefix() {
    // A single-segment import (`import Foundation`) has no `.` in the prefix —
    // the rule must not form `Foundation.Foo` and bind it.
    let lookup = Lookup::new().with(sym(32, "Foo", "Foundation.Foo", "class", "src/a.cs"));
    let imports = vec![import("Foundation", None)];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

#[test]
fn declines_when_symbol_absent() {
    let lookup = Lookup::new().with(sym(
        33,
        "Other",
        "eShop.Other",
        "class",
        "src/other.cs",
    ));
    let imports = vec![import("*", Some("eShop.Catalog"))];
    assert_eq!(resolve(&lookup, "CatalogItem", imports), None);
}
