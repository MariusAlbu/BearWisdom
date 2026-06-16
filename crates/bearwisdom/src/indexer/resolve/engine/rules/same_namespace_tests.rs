use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn resolve(lookup: &Lookup, target: &str, ns: Option<&str>) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], ns);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match SameNamespaceRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_type_in_same_namespace() {
    // File declares `namespace eShop.Catalog`; a ref to `CatalogItem` finds
    // `eShop.Catalog.CatalogItem`.
    let lookup = Lookup::new().with(sym(
        20,
        "CatalogItem",
        "eShop.Catalog.CatalogItem",
        "class",
        "src/model.cs",
    ));
    assert_eq!(resolve(&lookup, "CatalogItem", Some("eShop.Catalog")), Some(20));
}

#[test]
fn declines_when_no_file_namespace() {
    let lookup = Lookup::new().with(sym(21, "Foo", "NS.Foo", "class", "src/a.cs"));
    assert_eq!(resolve(&lookup, "Foo", None), None);
}

#[test]
fn declines_when_qname_mismatch() {
    // The candidate lives in a different namespace.
    let lookup = Lookup::new().with(sym(22, "Bar", "Other.Bar", "class", "src/a.cs"));
    assert_eq!(resolve(&lookup, "Bar", Some("NS")), None);
}

#[test]
fn declines_when_namespace_is_empty_string() {
    let lookup = Lookup::new().with(sym(23, "Foo", ".Foo", "class", "src/a.cs"));
    assert_eq!(resolve(&lookup, "Foo", Some("")), None);
}
