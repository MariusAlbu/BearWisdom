use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn resolve(lookup: &Lookup, target: &str) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match AmbientNamespacePathRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_dotted_target_whose_qname_ends_with_suffix() {
    // `Express.Multer.File` ends with `.Express.Multer.File` when the package
    // prefix is absorbed — the leaf is `File`.
    let lookup = Lookup::new().with(sym(
        10,
        "File",
        "@types/multer.Express.Multer.File",
        "interface",
        "node_modules/@types/multer/index.d.ts",
    ));
    assert_eq!(resolve(&lookup, "Express.Multer.File"), Some(10));
}

#[test]
fn picks_shallowest_path_on_tie() {
    // Two candidates both end with `.Foo.Bar`; the one with fewer `/` wins.
    let lookup = Lookup::new()
        .with(sym(
            1,
            "Bar",
            "deep.pkg.sub.Foo.Bar",
            "class",
            "node_modules/deep/pkg/sub/index.d.ts",
        ))
        .with(sym(
            2,
            "Bar",
            "shallow.Foo.Bar",
            "class",
            "node_modules/shallow/index.d.ts",
        ));
    assert_eq!(resolve(&lookup, "Foo.Bar"), Some(2));
}

#[test]
fn declines_bare_target() {
    // A non-dotted name is not handled by this rule.
    let lookup = Lookup::new().with(sym(5, "File", "some.ns.File", "class", "src/a.ts"));
    assert_eq!(resolve(&lookup, "File"), None);
}

#[test]
fn declines_when_no_suffix_match() {
    // The only candidate has a qname that does NOT end with `.Other.File`.
    let lookup =
        Lookup::new().with(sym(6, "File", "pkg.File", "class", "node_modules/pkg/index.d.ts"));
    assert_eq!(resolve(&lookup, "Other.File"), None);
}
