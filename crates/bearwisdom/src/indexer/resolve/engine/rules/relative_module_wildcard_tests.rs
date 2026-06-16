use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: "*".to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

fn resolve_in(
    lookup: &Lookup,
    target: &str,
    file_path: &str,
    imports: Vec<ImportEntry>,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let mut fc = file_ctx(imports, None);
    fc.file_path = file_path.to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match RelativeModuleWildcardRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn super_glob_binds_symbol_in_sibling_module_file() {
    // `src/tests/foo_test.rs` with `use super::*` — parent module dir is
    // `src/tests`, so the candidate file is `src/tests.rs` or `src/tests/mod.rs`.
    let lookup =
        Lookup::new().with(sym(5, "parse", "parse", "function", "src/tests.rs"));
    let imports = vec![wildcard_import("super")];
    let got = resolve_in(&lookup, "parse", "src/tests/foo_test.rs", imports);
    assert_eq!(got, Some(5));
}

#[test]
fn crate_glob_binds_symbol_in_lib_rs() {
    // `src/foo/bar.rs` with `use crate::*` — crate root is `src/lib.rs`.
    let lookup = Lookup::new().with(sym(7, "helper", "helper", "function", "src/lib.rs"));
    let imports = vec![wildcard_import("crate")];
    let got = resolve_in(&lookup, "helper", "src/foo/bar.rs", imports);
    assert_eq!(got, Some(7));
}

#[test]
fn declines_non_relative_module() {
    // `use std::*` — not a relative module; rule must stay inert.
    let lookup = Lookup::new().with(sym(3, "HashMap", "HashMap", "class", "src/lib.rs"));
    let imports = vec![wildcard_import("std")];
    let got = resolve_in(&lookup, "HashMap", "src/main.rs", imports);
    assert_eq!(got, None);
}

#[test]
fn declines_dotted_target() {
    let lookup =
        Lookup::new().with(sym(9, "a", "a.b", "function", "src/lib.rs"));
    let imports = vec![wildcard_import("super")];
    let got = resolve_in(&lookup, "a.b", "src/foo.rs", imports);
    assert_eq!(got, None);
}
