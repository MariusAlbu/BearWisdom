use super::*;
use std::sync::Arc;

use crate::indexer::resolve::engine::contract::{
    ImportEntry, FileContext, RefContext, Symbol, SymbolLookup, SymbolSet,
};
use crate::indexer::resolve::engine::testkit::{accept_any, call_ref, source_symbol};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{DEFAULT_PROFILE, LanguageProfile};

static WWS_PROFILE: LanguageProfile = LanguageProfile {
    workspace_packages: true,
    wildcard_workspace_scope: true,
    ..DEFAULT_PROFILE
};

static WWS_SELF_PROFILE: LanguageProfile = LanguageProfile {
    workspace_packages: true,
    wildcard_workspace_scope: true,
    self_package_root: Some("crate"),
    qname_separator: "::",
    ..DEFAULT_PROFILE
};

// ---------------------------------------------------------------------------
// Minimal SymbolLookup that supports the workspace package methods.
// ---------------------------------------------------------------------------

struct WsLookup {
    /// Owned storage: (package_id, Symbol).
    symbols: Vec<(i64, Symbol)>,
    /// (specifier, package_id)
    packages: Vec<(&'static str, i64)>,
}

impl WsLookup {
    fn new() -> Self {
        Self {
            symbols: Vec::new(),
            packages: Vec::new(),
        }
    }

    fn with_pkg(mut self, specifier: &'static str, pkg_id: i64) -> Self {
        self.packages.push((specifier, pkg_id));
        self
    }

    fn with_sym(
        mut self,
        pkg_id: i64,
        id: i64,
        name: &'static str,
        kind: &'static str,
        file: &'static str,
    ) -> Self {
        self.symbols.push((
            pkg_id,
            Symbol {
                id,
                name: name.to_string(),
                qualified_name: name.to_string(),
                kind: kind.to_string(),
                visibility: None,
                file_path: Arc::from(file),
                scope_path: None,
                package_id: None,
                signature: None,
            },
        ));
        self
    }
}

impl SymbolLookup for WsLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        let refs: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|(_, sym)| sym.name == name)
            .map(|(_, sym)| sym)
            .collect();
        SymbolSet::Owned(refs)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        // Mirrors the production impl: `::` canonicalizes to `/` before the
        // deep-import peel, so a Rust `tantivy::schema` specifier walks the
        // same way an npm `@org/utils/sub` one does.
        let normalized;
        let specifier: &str = if specifier.contains("::") {
            normalized = specifier.replace("::", "/");
            &normalized
        } else {
            specifier
        };
        let mut path = specifier;
        loop {
            if let Some(&(_, pkg_id)) = self.packages.iter().find(|(s, _)| *s == path) {
                return Some(pkg_id);
            }
            match path.rfind('/') {
                Some(slash) => path = &path[..slash],
                None => return None,
            }
        }
    }

    fn symbols_in_package(&self, package_id: i64) -> SymbolSet<'_> {
        let refs: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|(pkg, _)| *pkg == package_id)
            .map(|(_, sym)| sym)
            .collect();
        SymbolSet::Owned(refs)
    }

    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.packages.iter().any(|(s, _)| *s == name)
    }
}

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: "*".to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

fn resolve(
    profile: &LanguageProfile,
    lookup: &WsLookup,
    target: &str,
    imports: Vec<ImportEntry>,
    file_package_id: Option<i64>,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "benches/bench_it.rs".to_string(),
        language: "rust".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &s,
        scope_chain: vec![],
        file_package_id,
    };
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile,
    };
    match WildcardWorkspacePackageRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_bare_name_under_deep_glob_via_sub_path() {
    // `use tantivy::collector::*;` — `TopDocs` is only reachable through a
    // `pub use` re-export nested another directory deeper (`file_path`
    // contains "collector" regardless of how many hops down it's declared).
    let lookup = WsLookup::new()
        .with_pkg("tantivy", 1)
        .with_sym(1, 42, "TopDocs", "struct", "src/collector/top_collector.rs")
        .with_sym(1, 99, "Schema", "struct", "src/schema/mod.rs");
    let imports = vec![wildcard_import("tantivy::collector")];
    assert_eq!(resolve(&WWS_PROFILE, &lookup, "TopDocs", imports, None), Some(42));
}

#[test]
fn binds_bare_name_under_crate_root_glob_with_no_sub_path() {
    // `use tantivy::*;` — bare crate-root glob, no sub-path to filter on; any
    // same-name kind-compatible member of the package is in scope.
    let lookup = WsLookup::new()
        .with_pkg("tantivy", 1)
        .with_sym(1, 7, "Index", "struct", "src/core/index.rs");
    let imports = vec![wildcard_import("tantivy")];
    assert_eq!(resolve(&WWS_PROFILE, &lookup, "Index", imports, None), Some(7));
}

#[test]
fn declines_when_gate_is_off() {
    let lookup = WsLookup::new()
        .with_pkg("tantivy", 1)
        .with_sym(1, 42, "TopDocs", "struct", "src/collector/top_collector.rs");
    let imports = vec![wildcard_import("tantivy::collector")];
    // DEFAULT_PROFILE has wildcard_workspace_scope = false.
    assert_eq!(resolve(&DEFAULT_PROFILE, &lookup, "TopDocs", imports, None), None);
}

#[test]
fn declines_when_no_wildcard_import() {
    let lookup = WsLookup::new()
        .with_pkg("tantivy", 1)
        .with_sym(1, 42, "TopDocs", "struct", "src/collector/top_collector.rs");
    let imports = vec![ImportEntry {
        imported_name: "TopDocs".to_string(),
        module_path: Some("tantivy::collector".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&WWS_PROFILE, &lookup, "TopDocs", imports, None), None);
}

#[test]
fn declines_ambiguous_candidates_across_globs() {
    // Two globs, each uniquely resolving `Thing` to a DIFFERENT symbol —
    // ambiguous, must stay unresolved rather than pick one.
    let lookup = WsLookup::new()
        .with_pkg("tantivy", 1)
        .with_pkg("othercrate", 2)
        .with_sym(1, 10, "Thing", "struct", "src/a.rs")
        .with_sym(2, 20, "Thing", "struct", "src/b.rs");
    let imports = vec![wildcard_import("tantivy"), wildcard_import("othercrate")];
    assert_eq!(resolve(&WWS_PROFILE, &lookup, "Thing", imports, None), None);
}

#[test]
fn declines_ambiguous_candidates_within_one_glob_sub_path() {
    // Two files both matching the same sub-path substring under one glob.
    let lookup = WsLookup::new()
        .with_pkg("tantivy", 1)
        .with_sym(1, 10, "TopDocs", "struct", "src/collector/top_collector.rs")
        .with_sym(1, 11, "TopDocs", "struct", "src/collector/other.rs");
    let imports = vec![wildcard_import("tantivy::collector")];
    assert_eq!(resolve(&WWS_PROFILE, &lookup, "TopDocs", imports, None), None);
}

#[test]
fn declines_relative_specifier() {
    let lookup = WsLookup::new()
        .with_pkg("./utils", 1)
        .with_sym(1, 5, "fn1", "function", "src/utils.rs");
    let imports = vec![wildcard_import("./utils")];
    assert_eq!(resolve(&WWS_PROFILE, &lookup, "fn1", imports, None), None);
}

#[test]
fn binds_via_self_package_root_crate_glob() {
    // `use crate::*;` from a file whose own package is 3 — `Thing` re-exported
    // at the crate root from a submodule, no sub-path to filter on.
    let lookup = WsLookup::new().with_sym(3, 55, "Thing", "struct", "src/thing.rs");
    let imports = vec![wildcard_import("crate")];
    assert_eq!(
        resolve(&WWS_SELF_PROFILE, &lookup, "Thing", imports, Some(3)),
        Some(55)
    );
}

#[test]
fn binds_via_self_package_root_nested_glob() {
    // `use crate::inner::*;` — nested self-package glob, sub-path "inner"
    // filters to the submodule's file.
    let lookup = WsLookup::new()
        .with_sym(3, 55, "InnerProbe", "struct", "src/inner.rs")
        .with_sym(3, 56, "OuterProbe", "struct", "src/outer.rs");
    let imports = vec![wildcard_import("crate::inner")];
    assert_eq!(
        resolve(&WWS_SELF_PROFILE, &lookup, "InnerProbe", imports, Some(3)),
        Some(55)
    );
}
