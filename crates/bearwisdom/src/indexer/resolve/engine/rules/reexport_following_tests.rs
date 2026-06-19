use super::*;
use crate::indexer::resolve::engine::contract::{ImportEntry, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

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
        }
    }
}

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
    fn generic_params(&self, qname: &str) -> Option<&[String]> {
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
