use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{EdgeKind, ExtractedRef};

/// Build an `ExtractedRef` for `target` with `module` set.
fn module_ref(target: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn resolve(lookup: &Lookup, target: &str, module: &str) -> Option<i64> {
    let r = module_ref(target, module);
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
    match RefModuleRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

fn resolve_no_module(lookup: &Lookup, target: &str) -> Option<i64> {
    use crate::indexer::resolve::engine::testkit::call_ref;
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
    match RefModuleRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_via_dot_qualified_name() {
    // `lists.map` → exact qname with `.` separator.
    let lookup = Lookup::new().with(sym(1, "map", "lists.map", "function", "src/lists.erl"));
    let got = resolve(&lookup, "map", "lists");
    assert_eq!(got, Some(1));
}

#[test]
fn binds_via_double_colon_separator() {
    // `dplyr::mutate` → exact qname with `::` separator.
    let lookup = Lookup::new().with(sym(2, "mutate", "dplyr::mutate", "function", "R/dplyr.R"));
    let got = resolve(&lookup, "mutate", "dplyr");
    assert_eq!(got, Some(2));
}

#[test]
fn declines_when_no_module_set() {
    let lookup = Lookup::new().with(sym(3, "map", "lists.map", "function", "src/lists.erl"));
    let got = resolve_no_module(&lookup, "map");
    assert_eq!(got, None);
}

#[test]
fn declines_when_qname_not_found() {
    let lookup = Lookup::new().with(sym(4, "filter", "lists.filter", "function", "src/lists.erl"));
    let got = resolve(&lookup, "map", "lists");
    assert_eq!(got, None);
}
