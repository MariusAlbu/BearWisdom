use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
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
    match FileImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_directly_imported_name() {
    let lookup =
        Lookup::new().with(sym(10, "Foo", "Foo", "class", "src/foo.ts"));
    let imports = vec![import("Foo", Some("./foo"))];
    assert_eq!(resolve(&lookup, "Foo", imports), Some(10));
}

#[test]
fn binds_via_alias_using_original_name() {
    // `import { Foo as Bar } from './foo'` — ref is `Bar`, lookup is `Foo`.
    let lookup =
        Lookup::new().with(sym(11, "Foo", "Foo", "class", "src/foo.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("./foo".to_string()),
        alias: Some("Bar".to_string()),
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Bar", imports), Some(11));
}

#[test]
fn declines_when_no_import_matches_target() {
    let lookup =
        Lookup::new().with(sym(12, "Baz", "Baz", "class", "src/baz.ts"));
    let imports = vec![import("Other", Some("./baz"))];
    assert_eq!(resolve(&lookup, "Baz", imports), None);
}

#[test]
fn declines_when_file_path_does_not_match_module() {
    let lookup =
        Lookup::new().with(sym(13, "Foo", "Foo", "class", "src/unrelated.ts"));
    let imports = vec![import("Foo", Some("./foo"))];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}
