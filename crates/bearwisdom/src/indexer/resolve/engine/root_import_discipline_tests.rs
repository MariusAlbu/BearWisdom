use std::collections::HashMap;
use std::sync::Arc;

use super::{apply, RootImportOutcome};
use crate::indexer::resolve::engine::cause::CauseKind;
use crate::indexer::resolve::engine::contract::{
    FileContext, ImportEntry, Symbol as ContractSymbol, SymbolLookup, SymbolSet,
};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::{ChainSegment, SegmentKind};

#[derive(Default)]
struct FakeLookup {
    by_name: HashMap<String, Vec<ContractSymbol>>,
    in_module: HashMap<String, Vec<ContractSymbol>>,
    module_files: HashMap<String, String>,
    packages: HashMap<String, i64>,
    package_symbols: HashMap<i64, Vec<ContractSymbol>>,
    return_types: HashMap<i64, TypeId>,
    external_names: Vec<String>,
    empty: Vec<ContractSymbol>,
    empty_pairs: Vec<(String, String)>,
}

fn sym(id: i64, name: &str, qname: &str, kind: &str, path: &str) -> ContractSymbol {
    ContractSymbol {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from(path),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for FakeLookup {}

impl SymbolLookup for FakeLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).unwrap_or(&self.empty))
    }
    fn by_qualified_name(&self, _: &str) -> Option<&ContractSymbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&ContractSymbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_module_from(&self, _source: &str, spec: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.in_module.get(spec).unwrap_or(&self.empty))
    }
    fn resolve_module_from(&self, _source: &str, spec: &str) -> Option<&str> {
        self.module_files.get(spec).map(String::as_str)
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_id_of(&self, id: i64) -> Option<TypeId> {
        self.return_types.get(&id).copied()
    }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_pairs
    }
    fn is_external_name(&self, name: &str, _: &str) -> bool {
        self.external_names.iter().any(|n| n == name)
    }
    fn workspace_package_id(&self, spec: &str) -> Option<i64> {
        let normalized = spec.replace("::", "/");
        let mut path = normalized.as_str();
        loop {
            if let Some(&id) = self.packages.get(path) {
                return Some(id);
            }
            match path.rfind('/') {
                Some(ix) => path = &path[..ix],
                None => return None,
            }
        }
    }
    fn symbols_in_package(&self, id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.package_symbols.get(&id).unwrap_or(&self.empty))
    }
}

fn seg(name: &str, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

fn ctx_with(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/a.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: None,
    }
}

fn imp(name: &str, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: false,
    }
}

#[test]
fn unimported_name_is_unconstrained() {
    let lookup = FakeLookup::default();
    let arena = TypeArena::new();
    let out = apply(&ctx_with(vec![]), &lookup, &arena, &seg("assert", false));
    assert!(matches!(out, RootImportOutcome::Unconstrained));
}

#[test]
fn scheme_prefixed_unlinked_import_denies_with_import_unlinked() {
    let mut lookup = FakeLookup::default();
    // A same-named stranger exists — the old fallbacks would hijack it.
    lookup.by_name.insert(
        "assert".into(),
        vec![sym(9, "assert", "URL.assert", "method", "src/url.ts")],
    );
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("assert", "node:assert/strict")]);
    match apply(&ctx, &lookup, &arena, &seg("assert", false)) {
        RootImportOutcome::Deny(c) => {
            assert_eq!(c.kind, CauseKind::ImportUnlinked);
            assert_eq!(c.symbol_id, None);
        }
        _ => panic!("scheme-prefixed unlinked import must deny"),
    }
}

#[test]
fn attested_external_head_denies_when_unlinked() {
    let mut lookup = FakeLookup::default();
    lookup.external_names.push("@tryghost/logging".into());
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("logging", "@tryghost/logging")]);
    match apply(&ctx, &lookup, &arena, &seg("logging", false)) {
        RootImportOutcome::Deny(c) => assert_eq!(c.kind, CauseKind::ImportUnlinked),
        _ => panic!("attested external must deny when unlinked"),
    }
}

#[test]
fn unattested_bare_specifier_stays_unconstrained() {
    let lookup = FakeLookup::default();
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("Database", "crate::db")]);
    let out = apply(&ctx, &lookup, &arena, &seg("Database", false));
    assert!(matches!(out, RootImportOutcome::Unconstrained));
}

#[test]
fn internally_linked_module_types_the_root() {
    let mut lookup = FakeLookup::default();
    lookup.in_module.insert(
        "./utils".into(),
        vec![sym(3, "helper", "utils.helper", "function", "src/utils.ts")],
    );
    let ty77 = TypeId(std::num::NonZeroU32::new(77).unwrap());
    lookup.return_types.insert(3, ty77);
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("helper", "./utils")]);
    match apply(&ctx, &lookup, &arena, &seg("helper", true)) {
        RootImportOutcome::Typed(recv) => assert_eq!(recv.ty, ty77),
        _ => panic!("linked module must type the root"),
    }
}

#[test]
fn relative_import_without_link_stays_unconstrained() {
    let lookup = FakeLookup::default();
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("helper", "./utils")]);
    let out = apply(&ctx, &lookup, &arena, &seg("helper", true));
    assert!(matches!(out, RootImportOutcome::Unconstrained));
}

#[test]
fn workspace_deep_import_types_from_the_package() {
    let mut lookup = FakeLookup::default();
    lookup.packages.insert("next".into(), 5);
    lookup.package_symbols.insert(
        5,
        vec![sym(21, "Link", "next.Link", "class", "packages/next/src/link.tsx")],
    );
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("Link", "next/link")]);
    match apply(&ctx, &lookup, &arena, &seg("Link", false)) {
        RootImportOutcome::Typed(recv) => assert_eq!(recv.id, Some(21)),
        _ => panic!("workspace deep import must type from the package"),
    }
}

#[test]
fn workspace_package_without_the_name_denies() {
    let mut lookup = FakeLookup::default();
    lookup.packages.insert("next".into(), 5);
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("Missing", "next/link")]);
    match apply(&ctx, &lookup, &arena, &seg("Missing", false)) {
        RootImportOutcome::Deny(c) => assert_eq!(c.kind, CauseKind::ImportUnlinked),
        _ => panic!("workspace package lacking the name must deny"),
    }
}

#[test]
fn uncaptured_return_on_scoped_callable_blames_the_callee() {
    let mut lookup = FakeLookup::default();
    lookup.in_module.insert(
        "./utils".into(),
        vec![sym(3, "helper", "utils.helper", "function", "src/utils.ts")],
    );
    let arena = TypeArena::new();
    let ctx = ctx_with(vec![imp("helper", "./utils")]);
    match apply(&ctx, &lookup, &arena, &seg("helper", true)) {
        RootImportOutcome::Deny(c) => {
            assert_eq!(c.kind, CauseKind::UncapturedReturn);
            assert_eq!(c.symbol_id, Some(3));
        }
        _ => panic!("scoped callable without captured return must blame it"),
    }
}

#[test]
fn wildcard_import_never_disciplines() {
    let mut lookup = FakeLookup::default();
    lookup.external_names.push("System".into());
    let mut entry = imp("System", "System");
    entry.is_wildcard = true;
    let arena = TypeArena::new();
    let out = apply(&ctx_with(vec![entry]), &lookup, &arena, &seg("System", false));
    assert!(matches!(out, RootImportOutcome::Unconstrained));
}
