use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn resolve(lookup: &Lookup, target: &str, source_qname: &str, scope_chain: Vec<String>) -> Option<i64> {
    let r = call_ref(target);
    // source_symbol already sets qualified_name = name, so passing the full
    // qname here is sufficient.
    let s = source_symbol(source_qname);
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, scope_chain);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match GenericParamRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_generic_param_declared_on_source_symbol() {
    // Source symbol `Repository` has generic param `T`; ref to `T` binds to `Repository`.
    let lookup = Lookup::new()
        .with(sym(1, "Repository", "Repository", "class", "src/repo.ts"))
        .with_generics("Repository", &["T"]);
    let got = resolve(&lookup, "T", "Repository", vec![]);
    assert_eq!(got, Some(1));
}

#[test]
fn binds_generic_param_from_enclosing_scope() {
    // The enclosing class `Container` has generic param `V`; a method inside it
    // references `V` — the rule finds it through the scope chain.
    let lookup = Lookup::new()
        .with(sym(2, "Container", "Container", "class", "src/container.ts"))
        .with_generics("Container", &["V"]);
    // source symbol is `Container.method`, scope chain includes `Container`.
    let got = resolve(&lookup, "V", "Container.method", vec!["Container".to_string()]);
    assert_eq!(got, Some(2));
}

#[test]
fn declines_dotted_target() {
    let lookup = Lookup::new()
        .with(sym(3, "Repository", "Repository", "class", "src/repo.ts"))
        .with_generics("Repository", &["T"]);
    // A dotted target cannot be a generic parameter.
    let got = resolve(&lookup, "Repository.T", "Repository", vec![]);
    assert_eq!(got, None);
}

#[test]
fn declines_when_param_not_declared() {
    let lookup = Lookup::new()
        .with(sym(4, "Foo", "Foo", "class", "src/foo.ts"))
        .with_generics("Foo", &["T"]);
    // `U` is not in Foo's declared generics.
    let got = resolve(&lookup, "U", "Foo", vec![]);
    assert_eq!(got, None);
}
