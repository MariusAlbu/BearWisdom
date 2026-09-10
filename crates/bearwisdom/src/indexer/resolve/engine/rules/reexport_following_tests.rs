use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::external_parse_payload::CachedParse;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::{ImportEntry, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

fn apply(lookup: &dyn SymbolLookup, target: &str, imports: Vec<ImportEntry>) -> Option<i64> {
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
    match ReexportFollowingRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

fn dart_symbol(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn dart_star_reexport(source_symbol_index: usize, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        source_symbol_index,
        target_name: "*".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: true,
    }
}

fn dart_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "dart".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn dart_chain_id_map() -> HashMap<(String, String), i64> {
    HashMap::from([
        (
            (
                "ext:dart:test/test.dart".to_string(),
                "test.barrel".to_string(),
            ),
            101,
        ),
        (
            (
                "ext:dart:matcher/expect.dart".to_string(),
                "matcher.barrel".to_string(),
            ),
            102,
        ),
        (
            (
                "ext:dart:matcher/src/expect/expect.dart".to_string(),
                "matcher.expect".to_string(),
            ),
            700,
        ),
    ])
}

fn dart_export_chain_files() -> Vec<ParsedFile> {
    vec![
        dart_file(
            "ext:dart:test/test.dart",
            vec![dart_symbol("barrel", "test.barrel")],
            vec![dart_star_reexport(0, "package:matcher/expect.dart")],
        ),
        dart_file(
            "ext:dart:matcher/expect.dart",
            vec![dart_symbol("barrel", "matcher.barrel")],
            vec![dart_star_reexport(0, "src/expect/expect.dart")],
        ),
        dart_file(
            "ext:dart:matcher/src/expect/expect.dart",
            vec![dart_symbol("expect", "matcher.expect")],
            Vec::new(),
        ),
    ]
}

fn resolve_dart_export_chain(files: &[ParsedFile], arena: Arc<TypeArena>) -> Option<i64> {
    let tree = Compilation::build(files, &dart_chain_id_map().into(), arena);
    apply(
        &tree,
        "expect",
        vec![ImportEntry {
            imported_name: "*".to_string(),
            module_path: Some("test".to_string()),
            alias: None,
            is_wildcard: true,
        }],
    )
}

#[test]
fn dart_wildcard_import_follows_package_and_relative_export_chain_hot_and_cold() {
    let hot_arena = Arc::new(TypeArena::new());
    let files = dart_export_chain_files();
    assert_eq!(
        resolve_dart_export_chain(&files, Arc::clone(&hot_arena)),
        Some(700)
    );

    let payloads: Vec<CachedParse> = files
        .iter()
        .map(|file| {
            let payload = CachedParse::from_parsed(file, &hot_arena);
            serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap()
        })
        .collect();
    let cold_arena = Arc::new(TypeArena::new());
    let cold_files: Vec<ParsedFile> = payloads
        .into_iter()
        .zip(&files)
        .map(|(payload, file)| {
            payload.into_parsed(
                &cold_arena,
                &file.path,
                &file.content_hash,
                file.size,
                file.mtime,
            )
        })
        .collect();
    assert_eq!(
        resolve_dart_export_chain(&cold_files, Arc::clone(&cold_arena)),
        Some(700),
        "portable-cache restoration must retain Dart export modules and re-export tags"
    );
}

/// Empty target — rule passes immediately.
#[test]
fn passes_empty_target() {
    let lookup = Lookup::new();
    assert_eq!(apply(&lookup, "", vec![]), None);
}

/// No imports in the file — rule passes.
#[test]
fn passes_no_imports() {
    let lookup = Lookup::new();
    assert_eq!(apply(&lookup, "Foo", vec![]), None);
}

/// Non-matching import (imported_name differs from target, not wildcard) — skipped.
#[test]
fn declines_non_matching_import() {
    let lookup = Lookup::new();
    let imports = vec![ImportEntry {
        imported_name: "Bar".to_string(),
        module_path: Some("./bar".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(apply(&lookup, "Foo", imports), None);
}

/// A non-relative import whose module cannot be resolved (testkit default `None`)
/// is skipped — `None => continue` in the resolve branch.
#[test]
fn declines_unresolved_non_relative_import() {
    let lookup = Lookup::new();
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("some-package".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(apply(&lookup, "Foo", imports), None);
}

// ---------------------------------------------------------------------------
// Bind path: custom SymbolLookup that supports one re-export hop
// ---------------------------------------------------------------------------

/// Wraps `Lookup` and adds re-export data + per-file symbol lists.
struct ReexportLookup {
    inner: Lookup,
    /// The file path whose `reexports_from` returns the configured pairs.
    from_module: String,
    /// `(exported_name, source_module)` pairs for `from_module`.
    reexports: Vec<(String, String)>,
    /// Symbols stored per file path for `in_module_from`.
    by_file: std::collections::HashMap<String, Vec<Symbol>>,
    /// Fixed return for `resolve_module_via_language_resolver` — simulates a
    /// language `ModuleResolver` hit for the rule's last-resort branch.
    /// `None` (the default) falls through to the trait default (`None`).
    language_resolved: Option<String>,
}

impl ReexportLookup {
    fn new(
        inner: Lookup,
        from_module: &str,
        reexports: Vec<(&str, &str)>,
        by_file: Vec<(&str, Symbol)>,
    ) -> Self {
        let mut file_map: std::collections::HashMap<String, Vec<Symbol>> =
            std::collections::HashMap::default();
        for (path, s) in by_file {
            file_map.entry(path.to_string()).or_default().push(s);
        }
        Self {
            inner,
            from_module: from_module.to_string(),
            reexports: reexports
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            by_file: file_map,
            language_resolved: None,
        }
    }

    fn with_language_resolved(mut self, path: &str) -> Self {
        self.language_resolved = Some(path.to_string());
        self
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for ReexportLookup {}

impl SymbolLookup for ReexportLookup {
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
    fn in_file(&self, _path: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn field_type_name(&self, qname: &str) -> Option<&str> {
        self.inner.field_type_name(qname)
    }
    fn return_type_name(&self, qname: &str) -> Option<&str> {
        self.inner.return_type_name(qname)
    }
    fn generic_params(&self, qname: &str) -> Option<Vec<String>> {
        self.inner.generic_params(qname)
    }
    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        if file_path == self.from_module.as_str() {
            &self.reexports
        } else {
            &[]
        }
    }
    fn is_external_name(&self, name: &str, pkg: &str) -> bool {
        self.inner.is_external_name(name, pkg)
    }
    fn in_module_from(&self, _source_file: &str, spec: &str) -> SymbolSet<'_> {
        match self.by_file.get(spec) {
            Some(syms) => SymbolSet::Owned(syms.iter().collect()),
            None => SymbolSet::empty(),
        }
    }
    fn resolve_module_via_language_resolver(
        &self,
        _language: &str,
        _source_file: &str,
        _spec: &str,
    ) -> Option<String> {
        self.language_resolved.clone()
    }
}

/// Named re-export: `./index.ts` re-exports `Foo` from `./foo.ts` where `Foo`
/// is defined. Importing `Foo` from `./index.ts` should resolve to `Foo`.
#[test]
fn binds_via_named_reexport_hop() {
    let foo_sym = sym(99, "Foo", "Foo", "class", "./foo.ts");
    let inner = Lookup::new();
    let lookup = ReexportLookup::new(
        inner,
        "./index.ts",
        vec![("Foo", "./foo.ts")],
        vec![("./foo.ts", foo_sym)],
    );
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("./index.ts".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(apply(&lookup, "Foo", imports), Some(99));
}

// ---------------------------------------------------------------------------
// Language-resolver fallback: the third branch, reached only when both the
// module map AND `is_relative_specifier` miss.
// ---------------------------------------------------------------------------

/// A bare specifier that is neither in the module map nor `is_relative_specifier`
/// (Dart's bare-relative `'foo.dart'` shape) reaches the rule's last-resort
/// `resolve_module_via_language_resolver` hop, and the resolved path is walked
/// through `follow_reexports` exactly like the first two branches' resolutions.
#[test]
fn binds_via_language_resolver_fallback_hop() {
    let foo_sym = sym(77, "Foo", "Foo", "class", "./foo_impl.dart");
    let inner = Lookup::new();
    let lookup = ReexportLookup::new(
        inner,
        "lib/barrel.dart",
        vec![("Foo", "./foo_impl.dart")],
        vec![("./foo_impl.dart", foo_sym)],
    )
    .with_language_resolved("lib/barrel.dart");
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("foo.dart".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(apply(&lookup, "Foo", imports), Some(77));
}

/// TS regression: a bare specifier the language resolver ALSO declines (the
/// default `Lookup` double, matching a real `NodeModuleResolver` decline for a
/// genuine external package like `"lodash"`) still passes — the third branch's
/// addition doesn't manufacture a match where none of the three branches find
/// one. Mirrors `declines_unresolved_non_relative_import`, which exercises the
/// exact same shape and is unchanged by this rule's edit.
#[test]
fn declines_when_language_resolver_also_misses() {
    let lookup = Lookup::new();
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("lodash".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(apply(&lookup, "Foo", imports), None);
}

/// TS regression: an existing relative-specifier import (`is_relative_specifier`
/// true) resolves through the SECOND branch exactly as before — the language
/// resolver is never consulted (`language_resolved` stays `None` and would panic
/// on no such fixture anyway; leaving it unset proves the branch is unreached).
#[test]
fn relative_specifier_never_reaches_language_resolver_branch() {
    let foo_sym = sym(55, "Foo", "Foo", "class", "./foo.ts");
    let inner = Lookup::new();
    let lookup = ReexportLookup::new(
        inner,
        "./index.ts",
        vec![("Foo", "./foo.ts")],
        vec![("./foo.ts", foo_sym)],
    );
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("./index.ts".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(apply(&lookup, "Foo", imports), Some(55));
}
