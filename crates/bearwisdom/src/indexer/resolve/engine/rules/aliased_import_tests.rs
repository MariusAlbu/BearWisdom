use super::*;
use crate::indexer::resolve::engine::contract::{
    FileContext, ImportEntry, Symbol, SymbolLookup, SymbolSet,
};
use crate::indexer::resolve::engine::testkit::{accept_any, call_ref, ref_ctx, source_symbol, sym};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use std::sync::Arc;

/// A minimal lookup that forwards to the testkit `Lookup` for symbol data and
/// resolves one fixed alias mapping.
struct AliasLookup {
    inner: crate::indexer::resolve::engine::testkit::Lookup,
    /// When a specifier starts with this prefix, rewrite it.
    alias_from: &'static str,
    alias_to: String,
}

impl AliasLookup {
    fn new(
        inner: crate::indexer::resolve::engine::testkit::Lookup,
        alias_from: &'static str,
        alias_to: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            alias_from,
            alias_to: alias_to.into(),
        }
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for AliasLookup {}

impl SymbolLookup for AliasLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.inner.by_name(name)
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        self.inner.by_qualified_name(qname)
    }
    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        self.inner.all_by_qualified_name(qname)
    }
    fn members_of(&self, parent: &str) -> SymbolSet<'_> {
        self.inner.members_of(parent)
    }
    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.inner.types_by_name(name)
    }
    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol> {
        self.inner.in_namespace(namespace)
    }
    fn has_in_namespace(&self, namespace: &str) -> bool {
        self.inner.has_in_namespace(namespace)
    }
    fn in_file(&self, path: &str) -> SymbolSet<'_> {
        self.inner.in_file(path)
    }
    fn field_type_name(&self, q: &str) -> Option<&str> {
        self.inner.field_type_name(q)
    }
    fn return_type_name(&self, q: &str) -> Option<&str> {
        self.inner.return_type_name(q)
    }
    fn generic_params(&self, q: &str) -> Option<Vec<String>> {
        self.inner.generic_params(q)
    }
    fn reexports_from(&self, path: &str) -> &[(String, String)] {
        self.inner.reexports_from(path)
    }
    fn is_external_name(&self, name: &str, lang: &str) -> bool {
        self.inner.is_external_name(name, lang)
    }
    fn resolve_path_alias(&self, _pkg: Option<i64>, specifier: &str) -> Option<String> {
        if specifier.starts_with(self.alias_from) {
            Some(self.alias_to.clone())
        } else {
            None
        }
    }
}

fn resolve_with_alias(
    lookup: &AliasLookup,
    target: &str,
    imports: Vec<ImportEntry>,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match AliasedImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_when_alias_rewrites_specifier() {
    let inner = crate::indexer::resolve::engine::testkit::Lookup::new()
        .with(sym(20, "utils", "utils", "function", "src/utils/index.ts"));
    let lookup = AliasLookup::new(inner, "@/", "src/utils/index.ts".to_string());
    let imports = vec![ImportEntry {
        imported_name: "utils".to_string(),
        module_path: Some("@/utils".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve_with_alias(&lookup, "utils", imports), Some(20));
}

#[test]
fn declines_when_alias_does_not_change_specifier() {
    // resolve_path_alias returns the same value → must decline.
    let inner = crate::indexer::resolve::engine::testkit::Lookup::new()
        .with(sym(21, "Foo", "Foo", "class", "src/foo.ts"));
    // alias_from won't match "./foo", so resolve_path_alias returns None → Pass.
    let lookup = AliasLookup::new(inner, "@/", "src/foo.ts".to_string());
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("./foo".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve_with_alias(&lookup, "Foo", imports), None);
}

#[test]
fn declines_when_no_import_matches_target() {
    let inner = crate::indexer::resolve::engine::testkit::Lookup::new()
        .with(sym(22, "Foo", "Foo", "class", "src/foo.ts"));
    let lookup = AliasLookup::new(inner, "@/", "src/foo.ts".to_string());
    let imports = vec![ImportEntry {
        imported_name: "Bar".to_string(),
        module_path: Some("@/foo".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve_with_alias(&lookup, "Foo", imports), None);
}

#[test]
fn binds_via_alias_import_with_path_rewrite() {
    // `import { Foo as F } from '@/foo'` — ref is `F`, alias rewrites to real path.
    let inner = crate::indexer::resolve::engine::testkit::Lookup::new()
        .with(sym(23, "Foo", "Foo", "class", "src/foo.ts"));
    let lookup = AliasLookup::new(inner, "@/", "src/foo.ts".to_string());
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("@/foo".to_string()),
        alias: Some("F".to_string()),
        is_wildcard: false,
    }];
    assert_eq!(resolve_with_alias(&lookup, "F", imports), Some(23));
}
