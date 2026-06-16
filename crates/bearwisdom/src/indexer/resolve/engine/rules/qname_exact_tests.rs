use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
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
    match QnameExactRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_dotted_target_to_exact_qname() {
    let lookup = Lookup::new().with(sym(
        9,
        "List",
        "Catalog.Service.List",
        "function",
        "src/svc.ts",
    ));
    assert_eq!(resolve(&lookup, "Catalog.Service.List"), Some(9));
}

#[test]
fn declines_bare_target() {
    let lookup = Lookup::new().with(sym(9, "List", "List", "function", "src/svc.ts"));
    assert_eq!(resolve(&lookup, "List"), None);
}

#[test]
fn declines_when_qname_absent() {
    let lookup = Lookup::new().with(sym(9, "List", "Other.List", "function", "src/svc.ts"));
    assert_eq!(resolve(&lookup, "Catalog.Service.List"), None);
}
