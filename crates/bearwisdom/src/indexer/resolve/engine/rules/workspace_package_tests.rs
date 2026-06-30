use super::*;
use std::sync::Arc;

use crate::indexer::resolve::engine::contract::{
    ImportEntry, FileContext, Symbol, SymbolLookup, SymbolSet,
};
use crate::indexer::resolve::engine::testkit::{accept_any, call_ref, ref_ctx, source_symbol};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{DEFAULT_PROFILE, LanguageProfile};

static WS_PROFILE: LanguageProfile = LanguageProfile {
    workspace_packages: true,
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
    /// (barrel_file_path, [(exported_name, source_module)]) re-export entries.
    reexports: Vec<(&'static str, Vec<(String, String)>)>,
    empty_reexports: Vec<(String, String)>,
}

impl WsLookup {
    fn new() -> Self {
        Self {
            symbols: Vec::new(),
            packages: Vec::new(),
            reexports: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }

    fn with_reexports(
        mut self,
        barrel: &'static str,
        entries: Vec<(&str, &str)>,
    ) -> Self {
        self.reexports.push((
            barrel,
            entries
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        ));
        self
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
    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.reexports
            .iter()
            .find(|(p, _)| *p == file_path)
            .map(|(_, e)| e.as_slice())
            .unwrap_or(self.empty_reexports.as_slice())
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        // Walk from the full specifier inward, honoring deep imports.
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

fn resolve(lookup: &WsLookup, target: &str, imports: Vec<ImportEntry>) -> Option<i64> {
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
        profile: &WS_PROFILE,
    };
    match WorkspacePackageRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

fn import(name: &str, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: false,
    }
}

#[test]
fn binds_named_import_from_workspace_package() {
    let lookup = WsLookup::new()
        .with_pkg("@org/utils", 1)
        .with_sym(1, 42, "createSlug", "function", "packages/utils/src/index.ts");
    let imports = vec![import("createSlug", "@org/utils")];
    assert_eq!(resolve(&lookup, "createSlug", imports), Some(42));
}

#[test]
fn declines_when_gate_is_off() {
    let lookup = WsLookup::new()
        .with_pkg("@org/utils", 1)
        .with_sym(1, 42, "createSlug", "function", "packages/utils/src/index.ts");
    let imports = vec![import("createSlug", "@org/utils")];
    let r = call_ref("createSlug");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    // Gate off — workspace_packages = false in DEFAULT_PROFILE.
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(WorkspacePackageRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn declines_relative_specifier() {
    let lookup = WsLookup::new()
        .with_pkg("./utils", 1)
        .with_sym(1, 5, "fn1", "function", "src/utils.ts");
    // `./utils` is relative — must not bind via workspace_package path.
    let imports = vec![import("fn1", "./utils")];
    assert_eq!(resolve(&lookup, "fn1", imports), None);
}

#[test]
fn declines_when_no_import_matches_target() {
    let lookup = WsLookup::new()
        .with_pkg("@org/utils", 1)
        .with_sym(1, 99, "other", "function", "packages/utils/src/index.ts");
    let imports = vec![import("createSlug", "@org/utils")];
    // Target is `other` but import only brings `createSlug` — no specifier found.
    assert_eq!(resolve(&lookup, "other", imports), None);
}

#[test]
fn binds_bare_declared_name_import_with_no_sub_path() {
    // `import { QueryClient } from '@tanstack/query-core'` — the specifier IS the
    // declared name (no deep sub-path), so `workspace_sub_path` returns None and
    // the no-sub-path fallback must bind the same-name symbol in the package.
    let lookup = WsLookup::new()
        .with_pkg("@tanstack/query-core", 10)
        .with_sym(
            10,
            13168,
            "QueryClient",
            "class",
            "packages/query-core/src/queryClient.ts",
        );
    let imports = vec![import("QueryClient", "@tanstack/query-core")];
    assert_eq!(resolve(&lookup, "QueryClient", imports), Some(13168));
}

#[test]
fn follows_export_star_to_sibling_package_declaration() {
    // `@tanstack/react-query` (pkg 15) re-exports `* from '@tanstack/query-core'`
    // through its index barrel; it declares no QueryClient itself. `query-core`
    // (pkg 10) re-exports `QueryClient from './queryClient'`, where the class
    // lives. A QueryClient ref scoped to react-query must thread both hops.
    let lookup = WsLookup::new()
        .with_pkg("@tanstack/react-query", 15)
        .with_pkg("@tanstack/query-core", 10)
        // The barrel symbols are what `workspace_pkg_barrels` scans; only the
        // file path + package attribution matter for barrel discovery.
        .with_sym(
            15,
            900,
            "useQuery",
            "function",
            "packages/react-query/src/index.ts",
        )
        .with_sym(
            10,
            901,
            "QueryCache",
            "class",
            "packages/query-core/src/index.ts",
        )
        .with_sym(
            10,
            13168,
            "QueryClient",
            "class",
            "packages/query-core/src/queryClient.ts",
        )
        .with_reexports(
            "packages/react-query/src/index.ts",
            vec![("*", "@tanstack/query-core")],
        )
        .with_reexports(
            "packages/query-core/src/index.ts",
            vec![("QueryClient", "./queryClient")],
        );
    let imports = vec![import("QueryClient", "@tanstack/react-query")];
    assert_eq!(resolve(&lookup, "QueryClient", imports), Some(13168));
}

#[test]
fn bare_declared_name_picks_the_package_def_over_a_sibling() {
    // QueryClient exists in two sibling packages; the import names query-core, so
    // the bind must scope to package 10's def even though package 19 also has one.
    let lookup = WsLookup::new()
        .with_pkg("@tanstack/query-core", 10)
        .with_pkg("@tanstack/solid-query", 19)
        .with_sym(
            10,
            13168,
            "QueryClient",
            "class",
            "packages/query-core/src/queryClient.ts",
        )
        .with_sym(
            19,
            15723,
            "QueryClient",
            "class",
            "packages/solid-query/src/QueryClient.ts",
        );
    let imports = vec![import("QueryClient", "@tanstack/query-core")];
    assert_eq!(resolve(&lookup, "QueryClient", imports), Some(13168));
}
