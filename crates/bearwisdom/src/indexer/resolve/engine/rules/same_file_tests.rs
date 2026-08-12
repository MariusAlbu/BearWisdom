use super::*;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, ref_ctx, source_symbol, sym,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

/// Minimal `SymbolLookup` that supports `in_file` lookups — the testkit's
/// `Lookup` always returns empty from `in_file`, so tests for `SameFileRule`
/// need their own double.
struct FileLookup {
    by_file: std::collections::HashMap<String, Vec<Symbol>>,
    by_name: std::collections::HashMap<String, Vec<Symbol>>,
    empty: Vec<Symbol>,
    empty_pairs: Vec<(String, String)>,
}

impl FileLookup {
    fn new() -> Self {
        Self {
            by_file: Default::default(),
            by_name: Default::default(),
            empty: Vec::new(),
            empty_pairs: Vec::new(),
        }
    }

    fn with(mut self, s: Symbol) -> Self {
        self.by_name.entry(s.name.clone()).or_default().push(s.clone());
        self.by_file.entry(s.file_path.to_string()).or_default().push(s);
        self
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for FileLookup {}

impl SymbolLookup for FileLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]))
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> { None }
    fn members_of(&self, _: &str) -> SymbolSet<'_> { SymbolSet::Borrowed(&self.empty) }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> { SymbolSet::Borrowed(&self.empty) }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> { Vec::new() }
    fn has_in_namespace(&self, _: &str) -> bool { false }
    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_file.get(file_path).map(|v| v.as_slice()).unwrap_or(&[]),
        )
    }
    fn field_type_name(&self, _: &str) -> Option<&str> { None }
    fn return_type_name(&self, _: &str) -> Option<&str> { None }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> { None }
    fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_pairs }
    fn is_external_name(&self, _: &str, _: &str) -> bool { false }
}

#[test]
fn binds_sibling_symbol_in_same_file() {
    let lookup = FileLookup::new().with(sym(10, "helper", "helper", "function", "src/main.ts"));
    let r = call_ref("helper");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext { file_ctx: &fc, ref_ctx: &rc, lookup: &lookup, kind: &kind, profile: &DEFAULT_PROFILE };
    match SameFileRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 10),
        _ => panic!("expected Resolved"),
    }
}

#[test]
fn declines_when_symbol_is_in_different_file() {
    let lookup = FileLookup::new().with(sym(10, "helper", "helper", "function", "src/other.ts"));
    let r = call_ref("helper");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext { file_ctx: &fc, ref_ctx: &rc, lookup: &lookup, kind: &kind, profile: &DEFAULT_PROFILE };
    assert!(matches!(SameFileRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn yields_to_explicit_import_binding_the_same_name() {
    let lookup = FileLookup::new().with(sym(20, "helper", "helper", "function", "src/main.ts"));
    let r = call_ref("helper");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![ImportEntry {
            imported_name: "helper".to_string(),
            module_path: Some("./utils".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext { file_ctx: &fc, ref_ctx: &rc, lookup: &lookup, kind: &kind, profile: &DEFAULT_PROFILE };
    assert!(matches!(SameFileRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn does_not_yield_to_a_renamed_import_whose_original_name_matches() {
    // `use m::helper as external_helper;` binds only `external_helper` — the
    // ORIGINAL name stays free for the file's own `helper` declaration.
    let lookup = FileLookup::new().with(sym(25, "helper", "helper", "function", "src/main.ts"));
    let r = call_ref("helper");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![ImportEntry {
            imported_name: "helper".to_string(),
            module_path: Some("./utils".to_string()),
            alias: Some("external_helper".to_string()),
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext { file_ctx: &fc, ref_ctx: &rc, lookup: &lookup, kind: &kind, profile: &DEFAULT_PROFILE };
    match SameFileRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 25),
        _ => panic!("expected Resolved: a rename's original name must not suppress the sibling"),
    }
}

#[test]
fn yields_to_a_renamed_import_on_its_bound_name() {
    // The ALIAS is the bound name — a same-file sibling must not shadow it.
    let lookup =
        FileLookup::new().with(sym(26, "external_helper", "external_helper", "function", "src/main.ts"));
    let r = call_ref("external_helper");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![ImportEntry {
            imported_name: "helper".to_string(),
            module_path: Some("./utils".to_string()),
            alias: Some("external_helper".to_string()),
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext { file_ctx: &fc, ref_ctx: &rc, lookup: &lookup, kind: &kind, profile: &DEFAULT_PROFILE };
    assert!(matches!(SameFileRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn does_not_yield_to_wildcard_import() {
    let lookup = FileLookup::new().with(sym(30, "helper", "helper", "function", "src/main.ts"));
    let r = call_ref("helper");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![ImportEntry {
            imported_name: "*".to_string(),
            module_path: Some("./utils".to_string()),
            alias: None,
            is_wildcard: true,
        }],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext { file_ctx: &fc, ref_ctx: &rc, lookup: &lookup, kind: &kind, profile: &DEFAULT_PROFILE };
    match SameFileRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 30),
        _ => panic!("expected Resolved: wildcard should not suppress sibling"),
    }
}
