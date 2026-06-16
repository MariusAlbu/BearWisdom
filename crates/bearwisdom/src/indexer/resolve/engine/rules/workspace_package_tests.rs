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
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
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
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
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
