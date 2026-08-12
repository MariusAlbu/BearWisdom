use std::sync::Arc;

use super::*;
use crate::indexer::resolve::engine::contract::{
    FileContext, ImportEntry, Symbol, SymbolLookup, SymbolSet,
};
use crate::indexer::resolve::engine::testkit::{
    accept_any, ref_ctx, source_symbol, sym,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{EdgeKind, ExtractedRef};

// ---------------------------------------------------------------------------
// A lookup double that supports `in_module_from`, `resolve_path_alias`, and
// `in_file` in addition to the basic by-name / by-qname maps.
// ---------------------------------------------------------------------------

struct ComponentLookup {
    inner: crate::indexer::resolve::engine::testkit::Lookup,
    /// `(module_path, symbol)` pairs served by `in_module_from`.
    module_symbols: Vec<(String, Symbol)>,
    /// `(specifier, resolved_path)` pairs served by `resolve_path_alias`.
    aliases: Vec<(String, String)>,
    /// `(file_path, symbol)` pairs served by `in_file`.
    file_symbols: Vec<(String, Symbol)>,
}

impl ComponentLookup {
    fn new() -> Self {
        Self {
            inner: crate::indexer::resolve::engine::testkit::Lookup::new(),
            module_symbols: Vec::new(),
            aliases: Vec::new(),
            file_symbols: Vec::new(),
        }
    }

    fn with_sym(mut self, s: Symbol) -> Self {
        self.inner = self.inner.with(s);
        self
    }

    fn with_module_sym(mut self, module: &str, s: Symbol) -> Self {
        self.module_symbols.push((module.to_string(), s));
        self
    }

    fn with_alias(mut self, from: &str, to: &str) -> Self {
        self.aliases.push((from.to_string(), to.to_string()));
        self
    }

    fn with_file_sym(mut self, file: &str, s: Symbol) -> Self {
        self.file_symbols.push((file.to_string(), s));
        self
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for ComponentLookup {}

impl SymbolLookup for ComponentLookup {
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
    fn in_namespace(&self, ns: &str) -> Vec<&Symbol> {
        self.inner.in_namespace(ns)
    }
    fn has_in_namespace(&self, ns: &str) -> bool {
        self.inner.has_in_namespace(ns)
    }
    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        SymbolSet::Owned(
            self.file_symbols
                .iter()
                .filter(|(f, _)| f == file_path)
                .map(|(_, s)| s)
                .collect(),
        )
    }
    fn in_module_from(&self, _source_file: &str, spec: &str) -> SymbolSet<'_> {
        SymbolSet::Owned(
            self.module_symbols
                .iter()
                .filter(|(m, _)| m == spec)
                .map(|(_, s)| s)
                .collect(),
        )
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
        self.aliases
            .iter()
            .find(|(from, _)| from == specifier)
            .map(|(_, to)| to.clone())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn calls_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn vue_sym(id: i64, name: &str) -> Symbol {
    sym(id, name, name, "component", &format!("src/components/{name}.vue"))
}

fn make_fc(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/views/Page.vue".to_string(),
        language: "vue".to_string(),
        imports,
        file_namespace: None,
    }
}

fn run(lookup: &dyn SymbolLookup, target: &str, imports: Vec<ImportEntry>) -> LookupResult {
    let r = calls_ref(target);
    let s = source_symbol("render");
    let fc = make_fc(imports);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    ComponentImportRule.apply(&ctx)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn binds_via_by_name_when_file_path_matches_module() {
    let sym = vue_sym(1, "MyCard");
    let lookup = ComponentLookup::new().with_sym(sym);
    let imports = vec![ImportEntry {
        imported_name: "MyCard".to_string(),
        module_path: Some("./components/MyCard.vue".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    match run(&lookup, "MyCard", imports) {
        LookupResult::Resolved(r) => assert_eq!(r.target_symbol_id, 1),
        v => panic!("expected Resolved, got {v:?}"),
    }
}

#[test]
fn binds_via_alias_lookup_name() {
    // Import uses an alias: `import { Foo as MyCard } from '...'`
    let sym = Symbol {
        id: 2,
        name: "Foo".to_string(),
        qualified_name: "Foo".to_string(),
        kind: "component".to_string(),
        visibility: None,
        file_path: Arc::from("src/components/Foo.vue"),
        scope_path: None,
        package_id: None,
        signature: None,
    };
    let lookup = ComponentLookup::new().with_sym(sym);
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("./components/Foo.vue".to_string()),
        alias: Some("MyCard".to_string()),
        is_wildcard: false,
    }];
    match run(&lookup, "MyCard", imports) {
        LookupResult::Resolved(r) => assert_eq!(r.target_symbol_id, 2),
        v => panic!("expected Resolved, got {v:?}"),
    }
}

#[test]
fn binds_via_module_symbols_when_unique() {
    let s = vue_sym(3, "Banner");
    let lookup = ComponentLookup::new().with_module_sym("./Banner.vue", s);
    let imports = vec![ImportEntry {
        imported_name: "Banner".to_string(),
        module_path: Some("./Banner.vue".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    // by_name returns nothing; falls through to in_module_from
    match run(&lookup, "Banner", imports) {
        LookupResult::Resolved(r) => assert_eq!(r.target_symbol_id, 3),
        v => panic!("expected Resolved, got {v:?}"),
    }
}

#[test]
fn declines_when_module_symbols_are_ambiguous() {
    let s1 = vue_sym(4, "Banner");
    let s2 = Symbol {
        id: 5,
        name: "Footer".to_string(),
        qualified_name: "Footer".to_string(),
        kind: "component".to_string(),
        visibility: None,
        file_path: Arc::from("src/Banner.vue"),
        scope_path: None,
        package_id: None,
        signature: None,
    };
    let lookup = ComponentLookup::new()
        .with_module_sym("./Banner.vue", s1)
        .with_module_sym("./Banner.vue", s2);
    let imports = vec![ImportEntry {
        imported_name: "Banner".to_string(),
        module_path: Some("./Banner.vue".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert!(matches!(run(&lookup, "Banner", imports), LookupResult::Pass));
}

#[test]
fn binds_via_path_alias_in_file() {
    let file_sym = Symbol {
        id: 6,
        name: "Card".to_string(),
        qualified_name: "Card".to_string(),
        kind: "component".to_string(),
        visibility: None,
        file_path: Arc::from("src/components/Card.vue"),
        scope_path: None,
        package_id: None,
        signature: None,
    };
    let lookup = ComponentLookup::new()
        .with_alias("@/Card", "src/components/Card.vue")
        .with_file_sym("src/components/Card.vue", file_sym);
    let imports = vec![ImportEntry {
        imported_name: "Card".to_string(),
        module_path: Some("@/Card".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    match run(&lookup, "Card", imports) {
        LookupResult::Resolved(r) => assert_eq!(r.target_symbol_id, 6),
        v => panic!("expected Resolved, got {v:?}"),
    }
}

#[test]
fn passes_when_target_is_lowercase() {
    let lookup = ComponentLookup::new();
    assert!(matches!(run(&lookup, "div", vec![]), LookupResult::Pass));
}

#[test]
fn passes_when_no_matching_import() {
    let s = vue_sym(7, "MyCard");
    let lookup = ComponentLookup::new().with_sym(s);
    // The import names something else.
    let imports = vec![ImportEntry {
        imported_name: "OtherComp".to_string(),
        module_path: Some("./OtherComp.vue".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert!(matches!(run(&lookup, "MyCard", imports), LookupResult::Pass));
}

#[test]
fn passes_for_non_calls_edge() {
    let s = vue_sym(8, "MyCard");
    let lookup = ComponentLookup::new().with_sym(s);
    let r = ExtractedRef {
        kind: EdgeKind::TypeRef,
        target_name: "MyCard".to_string(),
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let source = source_symbol("caller");
    let fc = make_fc(vec![ImportEntry {
        imported_name: "MyCard".to_string(),
        module_path: Some("./MyCard.vue".to_string()),
        alias: None,
        is_wildcard: false,
    }]);
    let rc = ref_ctx(&r, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(ComponentImportRule.apply(&ctx), LookupResult::Pass));
}
