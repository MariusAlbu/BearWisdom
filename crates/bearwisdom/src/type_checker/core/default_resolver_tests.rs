// =============================================================================
// type_checker/core/default_resolver_tests.rs — unit tests for DefaultResolver.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::profile::language_profile::{
    AliasDecode, NameNormalization, NamespaceScope, NormSpec,
};
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Synthetic SymbolLookup — drives the strategies without spinning up a DB.
// ---------------------------------------------------------------------------

struct Lookup {
    empty: Vec<SymbolInfo>,
    empty_pairs: Vec<(String, String)>,
    by_name: FxHashMap<String, Vec<SymbolInfo>>,
    by_qname: FxHashMap<String, SymbolInfo>,
    ambient_paths: Vec<String>,
    reexport: FxHashMap<(String, String, String), i64>,
    in_file: FxHashMap<String, Vec<SymbolInfo>>,
    members: FxHashMap<String, Vec<SymbolInfo>>,
    parents: FxHashMap<String, String>,
    path_aliases: FxHashMap<String, String>,
    generics: FxHashMap<String, Vec<String>>,
    /// file_path → [(exported_name, source_module_spec)] (the per-file re-export map)
    reexports_map: FxHashMap<String, Vec<(String, String)>>,
    /// module_spec → resolved file_path
    module_files: FxHashMap<String, String>,
    /// qname → every overload stored under it (declaration-merging)
    by_qname_all: FxHashMap<String, Vec<SymbolInfo>>,
    /// raw selector → class qname
    selectors: FxHashMap<String, String>,
    /// workspace declared_name → package_id
    workspace_pkgs: FxHashMap<String, i64>,
    /// package_id → symbols
    pkg_symbols: FxHashMap<i64, Vec<SymbolInfo>>,
}

impl Lookup {
    fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_pairs: Vec::new(),
            by_name: Default::default(),
            by_qname: Default::default(),
            ambient_paths: Vec::new(),
            reexport: Default::default(),
            in_file: Default::default(),
            members: Default::default(),
            parents: Default::default(),
            path_aliases: Default::default(),
            generics: Default::default(),
            reexports_map: Default::default(),
            module_files: Default::default(),
            by_qname_all: Default::default(),
            selectors: Default::default(),
            workspace_pkgs: Default::default(),
            pkg_symbols: Default::default(),
        }
    }
    /// Register an additional overload under an existing qname (declaration
    /// merging). The first `with` already seeded `by_qname`; this appends the
    /// overload to the all-overloads slice.
    fn with_overload(mut self, sym: SymbolInfo) -> Self {
        self.by_qname_all
            .entry(sym.qualified_name.clone())
            .or_default()
            .push(sym.clone());
        self.by_name.entry(sym.name.clone()).or_default().push(sym);
        self
    }
    fn with_selector(mut self, raw: &str, class_qname: &str) -> Self {
        self.selectors
            .insert(raw.to_string(), class_qname.to_string());
        self
    }
    fn with_workspace_pkg(mut self, declared: &str, id: i64) -> Self {
        self.workspace_pkgs.insert(declared.to_string(), id);
        self
    }
    fn with_pkg_symbol(mut self, id: i64, sym: SymbolInfo) -> Self {
        self.pkg_symbols.entry(id).or_default().push(sym);
        self
    }
    fn with_reexport_entry(mut self, file: &str, name: &str, source: &str) -> Self {
        self.reexports_map
            .entry(file.to_string())
            .or_default()
            .push((name.to_string(), source.to_string()));
        self
    }
    fn with_module_file(mut self, spec: &str, file: &str) -> Self {
        self.module_files.insert(spec.to_string(), file.to_string());
        self
    }
    fn with_generics(mut self, qname: &str, params: &[&str]) -> Self {
        self.generics.insert(
            qname.to_string(),
            params.iter().map(|s| s.to_string()).collect(),
        );
        self
    }
    fn with_in_file(mut self, file: &str, sym: SymbolInfo) -> Self {
        self.in_file.entry(file.to_string()).or_default().push(sym);
        self
    }
    fn with_member(mut self, parent_qname: &str, sym: SymbolInfo) -> Self {
        self.members
            .entry(parent_qname.to_string())
            .or_default()
            .push(sym);
        self
    }
    fn with_parent(mut self, child_qname: &str, parent_qname: &str) -> Self {
        self.parents
            .insert(child_qname.to_string(), parent_qname.to_string());
        self
    }
    fn with_path_alias(mut self, from: &str, to: &str) -> Self {
        self.path_aliases.insert(from.to_string(), to.to_string());
        self
    }
    fn with(mut self, sym: SymbolInfo) -> Self {
        self.by_name
            .entry(sym.name.clone())
            .or_default()
            .push(sym.clone());
        self.by_qname.insert(sym.qualified_name.clone(), sym);
        self
    }
    fn with_ambient(mut self, path: &str) -> Self {
        self.ambient_paths.push(path.to_string());
        self
    }
    fn with_reexport(mut self, target: &str, prefix: &str, module: &str, sym_id: i64) -> Self {
        self.reexport.insert(
            (target.to_string(), prefix.to_string(), module.to_string()),
            sym_id,
        );
        self
    }
}

impl SymbolLookup for Lookup {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.get(qname)
    }
    fn members_of(&self, parent: &str) -> &[SymbolInfo] {
        self.members
            .get(parent)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo> {
        let prefix = format!("{namespace}.");
        self.by_qname
            .values()
            .filter(|s| s.qualified_name.starts_with(&prefix))
            .collect()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, path: &str) -> &[SymbolInfo] {
        self.in_file
            .get(path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
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
    fn generic_params(&self, qname: &str) -> Option<&[String]> {
        self.generics.get(qname).map(|v| v.as_slice())
    }
    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.reexports_map
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty_pairs)
    }
    fn resolve_module_from(&self, _source_file: &str, spec: &str) -> Option<&str> {
        self.module_files.get(spec).map(|s| s.as_str())
    }
    fn in_module_from(&self, _source_file: &str, spec: &str) -> &[SymbolInfo] {
        match self.module_files.get(spec) {
            Some(file) => self.in_file(file),
            None => &self.empty,
        }
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn is_ambient_path(&self, path: &str) -> bool {
        self.ambient_paths.iter().any(|p| p == path)
    }
    fn resolve_external_reexport(
        &self,
        target_name: &str,
        chain_prefix: &str,
        module_path: &str,
    ) -> Option<i64> {
        self.reexport
            .get(&(
                target_name.to_string(),
                chain_prefix.to_string(),
                module_path.to_string(),
            ))
            .copied()
    }
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents.get(class_qname).map(|s| s.as_str())
    }
    fn resolve_path_alias(&self, _: Option<i64>, specifier: &str) -> Option<String> {
        self.path_aliases.get(specifier).cloned()
    }
    fn all_by_qualified_name(&self, qname: &str) -> &[SymbolInfo] {
        if let Some(all) = self.by_qname_all.get(qname) {
            return all.as_slice();
        }
        std::slice::from_ref(match self.by_qname.get(qname) {
            Some(s) => s,
            None => return &self.empty,
        })
    }
    fn selector_qname(&self, raw_selector: &str) -> Option<&str> {
        self.selectors.get(raw_selector).map(|s| s.as_str())
    }
    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        if let Some(id) = self.workspace_pkgs.get(specifier) {
            return Some(*id);
        }
        // Deep-import prefix walk.
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if let Some(id) = self.workspace_pkgs.get(path) {
                return Some(*id);
            }
        }
        None
    }
    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.workspace_pkgs.contains_key(name)
    }
    fn symbols_in_package(&self, package_id: i64) -> &[SymbolInfo] {
        self.pkg_symbols
            .get(&package_id)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn sym(id: i64, name: &str, qname: &str, kind: &str, file: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from(file),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

fn import(name: &str, module: Option<&str>) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: module.map(|s| s.to_string()),
        alias: None,
        is_wildcard: false,
    }
}

fn aliased_import(name: &str, alias: &str, module: Option<&str>) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: module.map(|s| s.to_string()),
        alias: Some(alias.to_string()),
        is_wildcard: false,
    }
}

fn file_ctx(imports: Vec<ImportEntry>, ns: Option<&str>) -> FileContext {
    FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: ns.map(|s| s.to_string()),
    }
}

fn extracted_call(target: &str) -> ExtractedRef {
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

fn extracted_call_with_module(target: &str, module: &str) -> ExtractedRef {
    let mut r = extracted_call(target);
    r.module = Some(module.to_string());
    r
}

fn extracted_call_with_chain(target: &str, segments: &[&str]) -> ExtractedRef {
    let mut r = extracted_call(target);
    r.chain = Some(MemberChain {
        segments: segments
            .iter()
            .map(|s| ChainSegment {
                name: s.to_string(),
                node_kind: "test".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            })
            .collect(),
    });
    r
}

fn source_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
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

fn ref_ctx<'a>(
    r: &'a ExtractedRef,
    sym: &'a ExtractedSymbol,
    scope_chain: Vec<String>,
) -> RefContext<'a> {
    RefContext {
        extracted_ref: r,
        source_symbol: sym,
        scope_chain,
        file_package_id: None,
    }
}

fn accept_any(_: EdgeKind, _: &str) -> bool {
    true
}

// ---------------------------------------------------------------------------
// Tests, one per strategy
// ---------------------------------------------------------------------------

#[test]
fn ref_module_matches_qname_form() {
    let lookup = Lookup::new().with(sym(7, "map", "List.map", "function", "lib/list.ml"));
    let r = extracted_call_with_module("map", "List");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ref_module(&accept_any)
        .expect("module-qualified resolves");
    assert_eq!(resolved.target_symbol_id, 7);
    assert_eq!(resolved.strategy, "default_ref_module");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn ref_module_falls_back_to_file_stem() {
    let lookup = Lookup::new().with(sym(11, "map", "map_fn", "function", "src/lists.erl"));
    let r = extracted_call_with_module("map", "lists");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ref_module(&accept_any)
        .expect("file-stem match resolves");
    assert_eq!(resolved.target_symbol_id, 11);
}

#[test]
fn ref_module_returns_none_without_module_field() {
    let lookup = Lookup::new().with(sym(1, "map", "List.map", "function", "lib/list.ml"));
    let r = extracted_call("map");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_ref_module(&accept_any).is_none());
}

#[test]
fn qname_exact_resolves_dotted_target() {
    let lookup = Lookup::new().with(sym(
        3,
        "List",
        "Catalog.Services.List",
        "function",
        "src/svc.cs",
    ));
    let r = extracted_call("Catalog.Services.List");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_qname_exact(false, &accept_any, NameNormalization::None)
        .expect("dotted target resolves");
    assert_eq!(resolved.target_symbol_id, 3);
    assert_eq!(resolved.strategy, "default_qname_exact");
}

#[test]
fn qname_exact_ignores_bare_target() {
    let lookup = Lookup::new().with(sym(3, "List", "Catalog.List", "function", "src/svc.cs"));
    let r = extracted_call("List");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_qname_exact(false, &accept_any, NameNormalization::None)
        .is_none());
}

#[test]
fn file_import_matches_relative_specifier() {
    let lookup = Lookup::new().with(sym(9, "Foo", "Foo", "class", "src/foo.ts"));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Foo", Some("./foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_import(&accept_any)
        .expect("import resolves");
    assert_eq!(resolved.target_symbol_id, 9);
    assert_eq!(resolved.strategy, "default_file_import");
}

#[test]
fn file_import_uses_alias_to_find_original_name() {
    let lookup = Lookup::new().with(sym(12, "Foo", "Foo", "class", "src/foo.ts"));
    let r = extracted_call("Bar");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![aliased_import("Foo", "Bar", Some("./foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_import(&accept_any)
        .expect("alias maps to original");
    assert_eq!(resolved.target_symbol_id, 12);
}

#[test]
fn component_import_binds_default_renamed_vue_component() {
    let lookup = Lookup::new()
        .with_module_file("./Button.vue", "src/Button.vue")
        .with_in_file(
            "src/Button.vue",
            sym(23, "Button", "Button", "class", "src/Button.vue"),
        );
    let r = extracted_call("NextButton");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("NextButton", Some("./Button.vue"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_component_import(&accept_any)
        .expect("renamed component import resolves through module file");
    assert_eq!(resolved.target_symbol_id, 23);
    assert_eq!(resolved.strategy, "default_component_import");
}

#[test]
fn component_import_binds_dotted_namespace_head() {
    let lookup = Lookup::new().with(sym(24, "Card", "Card", "class", "src/card.ts"));
    let r = extracted_call("Card.Root");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Card", Some("./card"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_component_import(&accept_any)
        .expect("dotted component tag resolves through imported head");
    assert_eq!(resolved.target_symbol_id, 24);
    assert_eq!(resolved.strategy, "default_component_import");
}

#[test]
fn component_import_declines_non_component_file_rename() {
    let lookup = Lookup::new()
        .with_module_file("./factory", "src/factory.ts")
        .with_in_file(
            "src/factory.ts",
            sym(25, "Factory", "Factory", "class", "src/factory.ts"),
        );
    let r = extracted_call("RenamedFactory");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("RenamedFactory", Some("./factory"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_component_import(&accept_any).is_none());
}

#[test]
fn namespace_import_expands_dotted_namespace() {
    let lookup = Lookup::new().with(sym(
        15,
        "CatalogItem",
        "eShop.Catalog.API.Model.CatalogItem",
        "class",
        "src/model.cs",
    ));
    let r = extracted_call("CatalogItem");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import(
            "eShop.Catalog.API.Model",
            Some("eShop.Catalog.API.Model"),
        )],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_namespace_import(&accept_any, NameNormalization::None)
        .expect("namespace import expands");
    assert_eq!(resolved.target_symbol_id, 15);
    assert_eq!(resolved.strategy, "default_namespace_import");
}

#[test]
fn ambient_namespace_path_matches_qname_suffix() {
    let lookup = Lookup::new().with(sym(
        21,
        "File",
        "@types/multer.Express.Multer.File",
        "interface",
        "ext:node_modules/@types/multer/index.d.ts",
    ));
    let r = extracted_call("Express.Multer.File");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ambient_namespace_path(&accept_any)
        .expect("qname suffix matches");
    assert_eq!(resolved.target_symbol_id, 21);
    assert_eq!(resolved.strategy, "default_ambient_namespace_path");
}

#[test]
fn same_namespace_resolves_when_file_namespace_set() {
    let lookup = Lookup::new().with(sym(
        30,
        "Category",
        "FamilyBudget.Api.Entities.Category",
        "class",
        "src/Category.cs",
    ));
    let r = extracted_call("Category");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], Some("FamilyBudget.Api.Entities"));
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_same_namespace(&accept_any, NameNormalization::None)
        .expect("same namespace resolves");
    assert_eq!(resolved.target_symbol_id, 30);
    assert_eq!(resolved.strategy, "default_same_namespace");
}

#[test]
fn same_namespace_boundary_check_rejects_partial_prefix() {
    let lookup = Lookup::new().with(sym(
        31,
        "Category",
        "FamilyBudget.Api.EntitiesOther.Category",
        "class",
        "src/Category.cs",
    ));
    let r = extracted_call("Category");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], Some("FamilyBudget.Api.Entities"));
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_same_namespace(&accept_any, NameNormalization::None)
        .is_none());
}

#[test]
fn imported_namespace_matches_qname_prefix() {
    let lookup = Lookup::new().with(sym(
        40,
        "Transaction",
        "FamilyBudget.Api.Entities.Transaction",
        "class",
        "src/Transaction.cs",
    ));
    let r = extracted_call("Transaction");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import("Entities", Some("FamilyBudget.Api.Entities"))],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_imported_namespace(&accept_any, NameNormalization::None)
        .expect("namespace prefix matches");
    assert_eq!(resolved.target_symbol_id, 40);
    assert_eq!(resolved.strategy, "default_imported_namespace");
}

#[test]
fn chain_prefix_uses_second_to_last_segment_against_imports() {
    let lookup = Lookup::new().with(sym(
        50,
        "resolve_and_write",
        "indexer::resolve::resolve_and_write",
        "function",
        "src/indexer/resolve/mod.rs",
    ));
    let r = extracted_call_with_chain("resolve_and_write", &["resolve", "resolve_and_write"]);
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import("resolve", Some("crate::indexer::resolve"))],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_chain_prefix(&accept_any)
        .expect("chain prefix resolves");
    assert_eq!(resolved.target_symbol_id, 50);
    assert_eq!(resolved.strategy, "default_chain_prefix");
}

#[test]
fn scope_visible_resolves_against_innermost_scope_first() {
    let lookup = Lookup::new()
        .with(sym(
            60,
            "helper",
            "outer.helper",
            "function",
            "src/outer.rs",
        ))
        .with(sym(
            61,
            "helper",
            "outer.inner.helper",
            "function",
            "src/outer.rs",
        ));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["outer.inner".to_string(), "outer".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_scope_visible(&accept_any, &["."], &[], NameNormalization::None)
        .expect("scope walk resolves");
    assert_eq!(resolved.target_symbol_id, 61, "innermost scope wins");
    assert_eq!(resolved.strategy, "default_scope_visible");
}

#[test]
fn scope_visible_resolves_with_profile_separator() {
    // A `::`-keyed scope member resolves when the profile separator is `::`.
    let lookup = Lookup::new().with(sym(90, "baz", "Foo::Bar::baz", "function", "src/foo.cpp"));
    let r = extracted_call("baz");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["Foo::Bar".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_scope_visible(&accept_any, &["."], &[], NameNormalization::None)
            .is_none(),
        "the `.` join cannot match a `::`-keyed qname"
    );
    let resolved = d
        .resolve_via_scope_visible(&accept_any, &[".", "::"], &[], NameNormalization::None)
        .expect("the `::` separator resolves the scope member");
    assert_eq!(resolved.target_symbol_id, 90);
    assert_eq!(resolved.strategy, "default_scope_visible");
}

#[test]
fn scope_visible_dot_separator_is_unaffected_by_extra_separators() {
    // Passing `["."]` only resolves a `.`-keyed member; the additive form is
    // a no-op for the universal `.`-keyed index.
    let lookup = Lookup::new().with(sym(
        91,
        "helper",
        "outer.helper",
        "function",
        "src/outer.rs",
    ));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["outer".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let with_extra = d
        .resolve_via_scope_visible(&accept_any, &[".", "::"], &[], NameNormalization::None)
        .expect("`.`-keyed member still resolves with an extra separator present");
    assert_eq!(with_extra.target_symbol_id, 91);
    let dot_only = d
        .resolve_via_scope_visible(&accept_any, &["."], &[], NameNormalization::None)
        .expect("`.`-keyed member resolves with `.` alone");
    assert_eq!(dot_only.target_symbol_id, 91);
}

#[test]
fn kind_compatible_filter_rejects_class_for_calls_when_strict() {
    fn strict_calls(edge: EdgeKind, sym_kind: &str) -> bool {
        match edge {
            EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor"),
            _ => true,
        }
    }
    let lookup = Lookup::new().with(sym(70, "Foo", "Foo", "class", "src/foo.ts"));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Foo", Some("./foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: strict_calls,
    };
    assert!(
        d.resolve_via_file_import(&strict_calls).is_none(),
        "strict kind filter rejects class for calls"
    );
}

#[test]
fn resolve_all_prefers_innermost_scope_over_imports() {
    // Two viable targets: one in the innermost scope, one via file import.
    // Canonical order says scope wins.
    let lookup = Lookup::new()
        .with(sym(
            80,
            "helper",
            "outer.helper",
            "function",
            "src/outer.rs",
        ))
        .with(sym(81, "helper", "Foo", "function", "ext:helper.ts"));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("helper", Some("./helper"))], None);
    let rc = ref_ctx(&r, &s, vec!["outer".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d.resolve_all().expect("one of the strategies resolves");
    assert_eq!(resolved.target_symbol_id, 80, "scope beats import");
    assert_eq!(resolved.strategy, "default_scope_visible");
}

#[test]
fn resolve_all_returns_none_with_no_strategy_match() {
    let lookup = Lookup::new();
    let r = extracted_call("nonexistent");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_all().is_none());
}

#[test]
fn unique_internal_name_resolves_single_candidate() {
    let lookup = Lookup::new().with(sym(
        200,
        "stdlib_lsame",
        "stdlib_lsame",
        "function",
        "src/blas/aux.fypp",
    ));
    let r = extracted_call("stdlib_lsame");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_unique_internal_name(&accept_any)
        .expect("single candidate resolves");
    assert_eq!(resolved.target_symbol_id, 200);
    assert_eq!(resolved.strategy, "default_unique_internal_name");
}

#[test]
fn unique_internal_name_refuses_when_multiple_candidates() {
    let lookup = Lookup::new()
        .with(sym(201, "doIt", "Foo.doIt", "function", "src/a.rs"))
        .with(sym(202, "doIt", "Bar.doIt", "function", "src/b.rs"));
    let r = extracted_call("doIt");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_unique_internal_name(&accept_any).is_none(),
        "ambiguity must not be guessed"
    );
}

#[test]
fn unique_internal_name_ignores_external_candidates() {
    let mut ext_sym = sym(
        203,
        "load",
        "vendor.load",
        "function",
        "ext:vendor/lib.d.ts",
    );
    ext_sym.file_path = Arc::from("ext:vendor/lib.d.ts");
    let lookup = Lookup::new()
        .with(sym(204, "load", "App.load", "function", "src/app.ts"))
        .with(ext_sym);
    let r = extracted_call("load");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_unique_internal_name(&accept_any)
        .expect("external excluded → one internal left");
    assert_eq!(resolved.target_symbol_id, 204);
}

#[test]
fn same_file_resolves_sibling_in_same_path() {
    let sibling = sym(110, "helper", "helper", "function", "src/main.ts");
    let lookup = Lookup::new().with_in_file("src/main.ts", sibling);
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_same_file(&accept_any, &[], NameNormalization::None)
        .expect("same-file sibling resolves");
    assert_eq!(resolved.target_symbol_id, 110);
    assert_eq!(resolved.strategy, "default_same_file");
}

#[test]
fn reexport_chain_resolves_direct_shape() {
    let sym = sym(
        90,
        "TSESTree",
        "@typescript-eslint/types.TSESTree",
        "namespace",
        "ext:node_modules/@typescript-eslint/types/dist/index.d.ts",
    );
    let lookup = Lookup::new().with(sym.clone()).with_reexport(
        "TSESTree",
        "TSESTree",
        "@typescript-eslint/utils",
        90,
    );
    let r = extracted_call("TSESTree");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import("TSESTree", Some("@typescript-eslint/utils"))],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_reexport_chain(&accept_any)
        .expect("reexport chain resolves");
    assert_eq!(resolved.target_symbol_id, 90);
    assert_eq!(resolved.strategy, "default_reexport_chain");
}

#[test]
fn reexport_chain_resolves_dotted_shape() {
    let sym_inner = sym(
        91,
        "Node",
        "@typescript-eslint/types.TSESTree.Node",
        "interface",
        "ext:node_modules/@typescript-eslint/types/dist/index.d.ts",
    );
    let lookup = Lookup::new().with(sym_inner).with_reexport(
        "Node",
        "TSESTree",
        "@typescript-eslint/utils",
        91,
    );
    let r = extracted_call("TSESTree.Node");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import("TSESTree", Some("@typescript-eslint/utils"))],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_reexport_chain(&accept_any)
        .expect("dotted reexport resolves");
    assert_eq!(resolved.target_symbol_id, 91);
}

#[test]
fn reexport_chain_skips_relative_specifiers() {
    let lookup = Lookup::new();
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Foo", Some("./foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_reexport_chain(&accept_any).is_none());
}

#[test]
fn reexport_following_resolves_pub_use_hop() {
    // foo re-exports Thing from bar (Rust `pub use crate::bar::Thing`); bar
    // defines Thing. A ref to Thing imported `from foo` resolves to bar's
    // definition by following the re-export hop.
    let lookup = Lookup::new()
        .with(sym(1, "Thing", "bar.Thing", "class", "bar.rs"))
        .with_in_file("bar.rs", sym(1, "Thing", "bar.Thing", "class", "bar.rs"))
        .with_reexport_entry("foo.rs", "Thing", "barmod")
        .with_module_file("foomod", "foo.rs")
        .with_module_file("barmod", "bar.rs");
    let r = extracted_call("Thing");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Thing", Some("foomod"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_reexport_following()
        .expect("pub-use re-export hop resolves");
    assert_eq!(resolved.target_symbol_id, 1);
    assert_eq!(resolved.strategy, "reexport_chain");
}

#[test]
fn reexport_following_resolves_wildcard_import_hop() {
    // Nim-style persisted shape: consumer `import foo` is stored as
    // `imported_name="*", module_path="foo"`; foo has `export results`;
    // results defines ok. The `*` convention carries wildcard semantics even
    // if a persistence path did not preserve the boolean flag.
    let lookup = Lookup::new()
        .with(sym(
            7,
            "ok",
            "ok",
            "function",
            "ext:nim:results/results.nim",
        ))
        .with_in_file(
            "ext:nim:results/results.nim",
            sym(7, "ok", "ok", "function", "ext:nim:results/results.nim"),
        )
        .with_reexport_entry("foo.nim", "*", "results")
        .with_module_file("foo", "foo.nim")
        .with_module_file("results", "ext:nim:results/results.nim");
    let r = extracted_call("ok");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "caller.nim".to_string(),
        language: "nim".to_string(),
        imports: vec![ImportEntry {
            imported_name: "*".to_string(),
            module_path: Some("foo".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_reexport_following()
        .expect("wildcard import follows module re-export");
    assert_eq!(resolved.target_symbol_id, 7);
    assert_eq!(resolved.strategy, "reexport_star");
}

#[test]
fn reexport_following_blocks_private_use() {
    // The soundness gate (Invariant #2): a PRIVATE `use crate::bar::Thing` in
    // foo is is_reexport=false, so foo has NO entry in the re-export map — even
    // though the caller imported Thing `from foo`. The hop must NOT fire;
    // following it would bind Thing through a module that merely imports it.
    let lookup = Lookup::new()
        .with(sym(1, "Thing", "bar.Thing", "class", "bar.rs"))
        .with_in_file("bar.rs", sym(1, "Thing", "bar.Thing", "class", "bar.rs"))
        // No re-export entry for foo — the private use never enters the map.
        .with_module_file("foomod", "foo.rs")
        .with_module_file("barmod", "bar.rs");
    let r = extracted_call("Thing");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Thing", Some("foomod"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_reexport_following().is_none(),
        "a private use must not forward a name through a re-export hop"
    );
}

#[test]
fn reexport_following_skips_external_module() {
    // The imported module resolves to an external (`ext:`) file → no internal
    // hop; cross-package re-exports are `resolve_via_reexport_chain`'s job.
    let lookup = Lookup::new()
        .with_reexport_entry("ext:node_modules/pkg/index.d.ts", "Thing", "deep")
        .with_module_file("pkg", "ext:node_modules/pkg/index.d.ts");
    let r = extracted_call("Thing");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Thing", Some("pkg"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_reexport_following().is_none(),
        "an import resolving to an ext: file must not trigger the internal hop"
    );
}

#[test]
fn ambient_package_prefers_declared_ambient_paths() {
    let lookup = Lookup::new()
        .with(sym(
            100,
            "describe",
            "vitest/globals.describe",
            "function",
            "ext:node_modules/vitest/globals.d.ts",
        ))
        .with(sym(
            101,
            "describe",
            "some.unrelated.describe",
            "function",
            "src/unrelated/describe.ts",
        ))
        .with_ambient("ext:node_modules/vitest/globals.d.ts");
    let r = extracted_call("describe");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ambient_package(&accept_any)
        .expect("ambient package preferred");
    assert_eq!(resolved.target_symbol_id, 100);
    assert_eq!(resolved.strategy, "default_ambient_package");
}

#[test]
fn ambient_namespace_prefix_strips_then_binds_ambient_symbol() {
    // `sys.concat` names the bare ambient `concat` under an alias prefix the
    // bicep profile declares. The earlier dotted strategies decline (the qname
    // is `bicep.builtins.concat`, not `*.sys.concat`); the prefix-strip retry
    // of the ambient-package probe then binds the bare symbol.
    let lookup = Lookup::new()
        .with(sym(
            200,
            "concat",
            "bicep.builtins.concat",
            "function",
            "ext:bicep-runtime:namespace.bicep",
        ))
        .with_ambient("ext:bicep-runtime:namespace.bicep");
    let r = extracted_call("sys.concat");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "main.bicep".to_string(),
        language: "bicep".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::bicep::BICEP_PROFILE)
        .expect("aliased ambient member binds after the prefix strip");
    assert_eq!(resolved.target_symbol_id, 200);
    assert_eq!(resolved.strategy, "default_ambient_package");
}

#[test]
fn ambient_namespace_prefix_bare_name_still_binds() {
    // A bare `concat` (no alias prefix) binds via the ordinary ambient-package
    // probe — the strip retry is additive, not a replacement.
    let lookup = Lookup::new()
        .with(sym(
            201,
            "concat",
            "bicep.builtins.concat",
            "function",
            "ext:bicep-runtime:namespace.bicep",
        ))
        .with_ambient("ext:bicep-runtime:namespace.bicep");
    let r = extracted_call("concat");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "main.bicep".to_string(),
        language: "bicep".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::bicep::BICEP_PROFILE)
        .expect("bare ambient member binds without any prefix");
    assert_eq!(resolved.target_symbol_id, 201);
}

#[test]
fn ambient_package_returns_none_without_declared_ambient_paths() {
    let lookup = Lookup::new().with(sym(102, "describe", "x.describe", "function", "src/x.ts"));
    let r = extracted_call("describe");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_ambient_package(&accept_any).is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_self_keyword — this/self/super against the enclosing type
// ---------------------------------------------------------------------------

#[test]
fn self_keyword_resolves_this_to_enclosing_type() {
    let lookup = Lookup::new().with(sym(300, "Bar", "com.app.Bar", "class", "src/Bar.java"));
    let r = extracted_call("this");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["com.app.Bar".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_self_keyword(&accept_any)
        .expect("this resolves to enclosing type");
    assert_eq!(resolved.target_symbol_id, 300);
    assert_eq!(resolved.strategy, "engine_self_keyword");
}

#[test]
fn self_keyword_resolves_super_to_parent() {
    let lookup = Lookup::new()
        .with(sym(310, "Bar", "com.app.Bar", "class", "src/Bar.java"))
        .with(sym(311, "Base", "com.app.Base", "class", "src/Base.java"))
        .with_parent("com.app.Bar", "com.app.Base");
    let r = extracted_call("super");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["com.app.Bar".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_self_keyword(&accept_any)
        .expect("super resolves to parent class");
    assert_eq!(resolved.target_symbol_id, 311);
}

#[test]
fn self_keyword_none_outside_a_type() {
    let lookup = Lookup::new();
    let r = extracted_call("this");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_self_keyword(&accept_any).is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_enclosing_member — inherited member of the enclosing type
// ---------------------------------------------------------------------------

#[test]
fn enclosing_member_resolves_inherited_field() {
    let lookup = Lookup::new()
        .with(sym(320, "Bar", "com.app.Bar", "class", "src/Bar.java"))
        .with(sym(321, "Base", "com.app.Base", "class", "src/Base.java"))
        .with_parent("com.app.Bar", "com.app.Base")
        .with_member(
            "com.app.Base",
            sym(322, "LOG", "com.app.Base.LOG", "field", "src/Base.java"),
        );
    let r = extracted_call("LOG");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["com.app.Bar".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_enclosing_member(&accept_any)
        .expect("inherited field resolves via inheritance climb");
    assert_eq!(resolved.target_symbol_id, 322);
    assert_eq!(resolved.strategy, "engine_enclosing_member");
}

#[test]
fn enclosing_member_none_outside_a_type() {
    let lookup = Lookup::new();
    let r = extracted_call("LOG");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_enclosing_member(&accept_any).is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_aliased_import — specifier rewritten through a path alias
// ---------------------------------------------------------------------------

#[test]
fn aliased_import_resolves_via_path_alias() {
    let lookup = Lookup::new()
        .with(sym(
            330,
            "helper",
            "helper",
            "function",
            "src/utils/helper.ts",
        ))
        .with_path_alias("@/utils/helper", "src/utils/helper");
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("helper", Some("@/utils/helper"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_aliased_import(&accept_any)
        .expect("aliased specifier resolves after rewrite");
    assert_eq!(resolved.target_symbol_id, 330);
    assert_eq!(resolved.strategy, "engine_aliased_import");
}

#[test]
fn aliased_import_none_without_rewrite() {
    // No alias registered → resolve_path_alias returns None → stay out,
    // leaving the raw-path case to resolve_via_file_import.
    let lookup = Lookup::new().with(sym(
        331,
        "helper",
        "helper",
        "function",
        "src/utils/helper.ts",
    ));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("helper", Some("@/utils/helper"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_aliased_import(&accept_any).is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_generic_param — self-declared params (string map)
// ---------------------------------------------------------------------------

#[test]
fn generic_param_resolves_self_declared_on_source_symbol() {
    // `OutputIt fill_n(OutputIt first, ...)` — `OutputIt` is declared on fill_n
    // itself, not an enclosing scope. The scope-chain path skips the source
    // symbol's own qname, so the self-declared string-map check must catch it.
    let lookup = Lookup::new()
        .with(sym(400, "fill_n", "fill_n", "function", "src/algo.cpp"))
        .with_generics("fill_n", &["OutputIt", "T"]);
    let r = extracted_call("OutputIt");
    let s = source_symbol("fill_n");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_generic_param()
        .expect("self-declared generic param resolves");
    assert_eq!(resolved.target_symbol_id, 400);
    assert_eq!(resolved.strategy, "engine_generic_param");
}

// ---------------------------------------------------------------------------
// resolve_via_ranked_candidates — multi-candidate disambiguation
// ---------------------------------------------------------------------------

fn sym_full(
    id: i64,
    name: &str,
    qname: &str,
    kind: &str,
    file: &str,
    visibility: Option<&str>,
    package_id: Option<i64>,
) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: visibility.map(|s| s.to_string()),
        file_path: Arc::from(file),
        scope_path: None,
        package_id,
        signature: None,
    }
}

struct WorkspaceLookup {
    inner: Lookup,
    workspace_packages: FxHashMap<String, i64>,
}

impl WorkspaceLookup {
    fn new(lookup: Lookup) -> Self {
        Self {
            inner: lookup,
            workspace_packages: Default::default(),
        }
    }
    fn with_workspace_package(mut self, specifier: &str, id: i64) -> Self {
        self.workspace_packages.insert(specifier.to_string(), id);
        self
    }
}

impl SymbolLookup for WorkspaceLookup {
    fn by_name(&self, n: &str) -> &[SymbolInfo] {
        self.inner.by_name(n)
    }
    fn by_qualified_name(&self, q: &str) -> Option<&SymbolInfo> {
        self.inner.by_qualified_name(q)
    }
    fn members_of(&self, p: &str) -> &[SymbolInfo] {
        self.inner.members_of(p)
    }
    fn types_by_name(&self, n: &str) -> &[SymbolInfo] {
        self.inner.types_by_name(n)
    }
    fn in_namespace(&self, n: &str) -> Vec<&SymbolInfo> {
        self.inner.in_namespace(n)
    }
    fn has_in_namespace(&self, n: &str) -> bool {
        self.inner.has_in_namespace(n)
    }
    fn in_file(&self, p: &str) -> &[SymbolInfo] {
        self.inner.in_file(p)
    }
    fn field_type_name(&self, q: &str) -> Option<&str> {
        self.inner.field_type_name(q)
    }
    fn return_type_name(&self, q: &str) -> Option<&str> {
        self.inner.return_type_name(q)
    }
    fn field_type_args(&self, q: &str) -> Option<&[String]> {
        self.inner.field_type_args(q)
    }
    fn generic_params(&self, q: &str) -> Option<&[String]> {
        self.inner.generic_params(q)
    }
    fn reexports_from(&self, p: &str) -> &[(String, String)] {
        self.inner.reexports_from(p)
    }
    fn is_external_name(&self, n: &str, l: &str) -> bool {
        self.inner.is_external_name(n, l)
    }
    fn is_ambient_path(&self, p: &str) -> bool {
        self.inner.is_ambient_path(p)
    }
    fn resolve_external_reexport(&self, t: &str, c: &str, m: &str) -> Option<i64> {
        self.inner.resolve_external_reexport(t, c, m)
    }
    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        self.workspace_packages.get(specifier).copied()
    }
}

fn ref_ctx_with_pkg<'a>(
    r: &'a ExtractedRef,
    sym: &'a ExtractedSymbol,
    pkg: Option<i64>,
) -> RefContext<'a> {
    RefContext {
        extracted_ref: r,
        source_symbol: sym,
        scope_chain: vec![],
        file_package_id: pkg,
    }
}

#[test]
fn ranked_returns_none_for_single_candidate() {
    // Only one candidate — the strict single-candidate strategies handle
    // this. Ranking must stay out so its attribution doesn't pollute
    // telemetry for trivially-resolved refs.
    let lookup = Lookup::new().with(sym(1, "X", "X", "class", "ext:lib/X.d.ts"));
    let r = extracted_call("X");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d.resolve_via_ranked_candidates(&accept_any).is_none());
}

#[test]
fn ranked_picks_same_workspace_package_over_external() {
    // Two `Foo` candidates: one internal (workspace pkg 42), one external.
    // The same-package bonus (+1000) dominates everything else.
    let lookup = Lookup::new()
        .with(sym_full(
            11,
            "Foo",
            "internal.Foo",
            "class",
            "src/internal/foo.ts",
            Some("public"),
            Some(42),
        ))
        .with(sym_full(
            22,
            "Foo",
            "ext.Foo",
            "class",
            "ext:idx:/cache/somelib/foo.d.ts",
            Some("public"),
            None,
        ));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx_with_pkg(&r, &s, Some(42));
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ranked_candidates(&accept_any)
        .expect("ranked picks workspace match");
    assert_eq!(resolved.target_symbol_id, 11);
    assert_eq!(resolved.strategy, "default_ranked_candidate");
}

#[test]
fn ranked_picks_imported_package_over_random_externals() {
    // Three external `expect` candidates from different packages. Caller
    // imports from `@types/jest` — workspace_package_id resolves that to id=7.
    // Only the jest-attributed candidate has package_id=7, so it wins.
    let inner = Lookup::new()
        .with(sym_full(
            1,
            "expect",
            "expect",
            "function",
            "ext:/cache/@types/jest/index.d.ts",
            Some("public"),
            Some(7),
        ))
        .with(sym_full(
            2,
            "expect",
            "expect",
            "function",
            "ext:/cache/@types/vitest/dist/index.d.ts",
            Some("public"),
            Some(8),
        ))
        .with(sym_full(
            3,
            "expect",
            "expect",
            "function",
            "ext:/cache/@types/chai/index.d.ts",
            Some("public"),
            Some(9),
        ));
    let lookup = WorkspaceLookup::new(inner).with_workspace_package("@types/jest", 7);

    let r = extracted_call("expect");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("jest", Some("@types/jest"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ranked_candidates(&accept_any)
        .expect("ranked picks imported package");
    assert_eq!(resolved.target_symbol_id, 1);
}

#[test]
fn ranked_returns_none_when_top_two_tie() {
    // Two indistinguishable external candidates — both public, same depth,
    // neither imported, no package match. Margin not met → stay None.
    let lookup = Lookup::new()
        .with(sym_full(
            101,
            "Foo",
            "Foo",
            "class",
            "ext:/cache/a/foo.d.ts",
            Some("public"),
            None,
        ))
        .with(sym_full(
            102,
            "Foo",
            "Foo",
            "class",
            "ext:/cache/b/foo.d.ts",
            Some("public"),
            None,
        ));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    // Both candidates score the same (public +50, external same depth);
    // the margin gate blocks the strategy from guessing.
    assert!(d.resolve_via_ranked_candidates(&accept_any).is_none());
}

#[test]
fn ranked_prefers_ambient_path() {
    // One ambient candidate (TS @types-style), one non-ambient. Ambient
    // bonus (+200) plus public-visibility tilts the win to the ambient one.
    let lookup = Lookup::new()
        .with(sym_full(
            50,
            "expect",
            "expect",
            "function",
            "ext:/cache/@types/jest/index.d.ts",
            Some("public"),
            None,
        ))
        .with(sym_full(
            51,
            "expect",
            "expect",
            "function",
            "ext:/cache/some-other/expect.d.ts",
            Some("public"),
            None,
        ))
        .with_ambient("ext:/cache/@types/jest/index.d.ts");
    let r = extracted_call("expect");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ranked_candidates(&accept_any)
        .expect("ranked picks ambient");
    assert_eq!(resolved.target_symbol_id, 50);
}

#[test]
fn ranked_penalises_private_external_candidates() {
    // One private external, one public external. The public one wins despite
    // identical paths — public+50 minus private-200 = 250-point gap.
    let lookup = Lookup::new()
        .with(sym_full(
            70,
            "Foo",
            "Foo",
            "class",
            "ext:/cache/somepkg/foo.d.ts",
            Some("private"),
            None,
        ))
        .with(sym_full(
            71,
            "Foo",
            "Foo",
            "class",
            "ext:/cache/somepkg/bar.d.ts",
            Some("public"),
            None,
        ));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ranked_candidates(&accept_any)
        .expect("ranked picks public");
    assert_eq!(resolved.target_symbol_id, 71);
}

#[test]
fn ranked_picks_via_qname_prefix_when_import_module_matches() {
    // Caller imports `lodash` — three lodash-named externals at different
    // qname paths. The one whose qname literally starts with `lodash.` wins
    // via the +300 prefix bonus.
    let lookup = Lookup::new()
        .with(sym_full(
            200,
            "map",
            "lodash.map",
            "function",
            "ext:/cache/@types/lodash/index.d.ts",
            Some("public"),
            None,
        ))
        .with(sym_full(
            201,
            "map",
            "rxjs.operators.map",
            "function",
            "ext:/cache/rxjs/operators.d.ts",
            Some("public"),
            None,
        ))
        .with(sym_full(
            202,
            "map",
            "other.helper.map",
            "function",
            "ext:/cache/other/helper.d.ts",
            Some("public"),
            None,
        ));
    let r = extracted_call("map");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("lodash", Some("lodash"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_ranked_candidates(&accept_any)
        .expect("ranked follows import");
    assert_eq!(resolved.target_symbol_id, 200);
}

#[test]
fn confidence_is_always_one_point_oh() {
    let lookup = Lookup::new()
        .with(sym(1, "map", "List.map", "function", "lib/list.ml"))
        .with(sym(2, "Foo", "Foo", "class", "src/foo.ts"));

    for (r, fc) in [
        (
            extracted_call_with_module("map", "List"),
            file_ctx(vec![], None),
        ),
        (
            extracted_call("Foo"),
            file_ctx(vec![import("Foo", Some("./foo"))], None),
        ),
    ] {
        let s = source_symbol("caller");
        let rc = ref_ctx(&r, &s, vec![]);
        let d = DefaultResolver {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind_compatible: accept_any,
        };
        let resolved = d
            .resolve_via_ref_module(&accept_any)
            .or_else(|| d.resolve_via_file_import(&accept_any))
            .expect("one of the strategies resolves");
        assert_eq!(resolved.confidence, 1.0, "deterministic, never decayed");
    }
}

#[test]
fn package_short_name_resolves_under_import_short_name() {
    // Go-shaped: `import "github.com/gin-gonic/gin"` brings short name `gin`,
    // and the member is keyed `gin.NewRouter`. A bare `NewRouter` ref whose
    // qualifier the extractor dropped resolves under `{imported_name}.{target}`.
    let lookup = Lookup::new().with(sym(
        5,
        "NewRouter",
        "gin.NewRouter",
        "function",
        "ext:go:gin/gin.go",
    ));
    let r = extracted_call("NewRouter");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import("gin", Some("github.com/gin-gonic/gin"))],
        Some("main"),
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_package_short_name(&accept_any, ".")
        .expect("resolves under the import short name");
    assert_eq!(resolved.target_symbol_id, 5);
    assert_eq!(resolved.strategy, "default_package_short_name");
}

#[test]
fn package_short_name_joins_with_profile_separator() {
    // Hare-shaped: `use fmt;` keys the package's members under `fmt::printfln`
    // (qname separator `::`, not `.`). A bare `printfln` ref whose qualifier
    // the extractor dropped resolves under `{imported_name}::{target}` when the
    // separator is threaded through from the profile.
    let lookup = Lookup::new().with(sym(
        9,
        "printfln",
        "fmt::printfln",
        "function",
        "ext:hare:fmt/fmt.ha",
    ));
    let r = extracted_call("printfln");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("fmt", Some("fmt"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    // The `.`-join form must NOT match the `::`-keyed qname.
    assert!(
        d.resolve_via_package_short_name(&accept_any, ".").is_none(),
        "dot join cannot reach a `::`-keyed package member"
    );
    let resolved = d
        .resolve_via_package_short_name(&accept_any, "::")
        .expect("resolves under the import short name with the `::` separator");
    assert_eq!(resolved.target_symbol_id, 9);
    assert_eq!(resolved.strategy, "default_package_short_name");
}

#[test]
fn package_short_name_resolves_aliased_import_via_last_segment() {
    // `import mygin "github.com/gin-gonic/gin"; mygin.Default()` — the symbol
    // stays keyed under the path's last segment (`gin.Default`), not the alias.
    let lookup = Lookup::new().with(sym(
        6,
        "Default",
        "gin.Default",
        "function",
        "ext:go:gin/gin.go",
    ));
    let r = extracted_call("Default");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![aliased_import(
            "mygin",
            "mygin",
            Some("github.com/gin-gonic/gin"),
        )],
        Some("main"),
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_package_short_name(&accept_any, ".")
        .expect("resolves via the path's last segment");
    assert_eq!(resolved.target_symbol_id, 6);
}

#[test]
fn package_short_name_off_under_default_ladder() {
    // The strategy is gated by ChainQualification::PackageShortName in the
    // ladder. The fn-pointer `resolve_all` path passes None, so the same fixture
    // does NOT resolve `gin.NewRouter` from a bare `NewRouter` through it.
    let lookup = Lookup::new().with(sym(
        5,
        "NewRouter",
        "gin.NewRouter",
        "function",
        "ext:go:gin/gin.go",
    ));
    let r = extracted_call("NewRouter");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![import("gin", Some("github.com/gin-gonic/gin"))],
        Some("main"),
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_all().is_none(),
        "default ladder (ChainQualification::None) must not reach package-short-name"
    );
}

// ---------------------------------------------------------------------------
// resolve_via_import_path — template-include resolution (data-driven)
// ---------------------------------------------------------------------------

use crate::type_checker::profile::language_profile::{CandidateDirs, ImportResolution, StemMatch};

/// A `FileContext` rooted at an arbitrary path (the `file_ctx` helper hardcodes
/// `src/main.ts`, which has no meaningful template directory).
fn file_ctx_at(path: &str) -> FileContext {
    FileContext {
        file_path: path.to_string(),
        language: "template".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    }
}

fn extracted_import(target: &str) -> ExtractedRef {
    let mut r = extracted_call(target);
    r.kind = EdgeKind::Imports;
    r
}

/// Base `ImportResolution` for tests — `.tmpl` ext, self-dir, exact stem
/// match, binds a `class`. Individual tests tweak only the field under test.
fn base_ir() -> ImportResolution {
    ImportResolution {
        extensions: &["tmpl"],
        candidate_dirs: CandidateDirs::SelfDir,
        index_files: &[],
        underscore_variant: false,
        kebab_variant: false,
        decline_leading_slash: false,
        stem_match: StemMatch::StemExact,
        bind_kind: "class",
        strategy_tag: "test_template_include",
    }
}

fn run_import_path(
    lookup: &Lookup,
    fc: &FileContext,
    r: &ExtractedRef,
    ir: &ImportResolution,
) -> Option<Resolution> {
    let s = source_symbol("caller");
    let rc = ref_ctx(r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: fc,
        ref_ctx: &rc,
        lookup,
        kind_compatible: accept_any,
    };
    d.resolve_via_import_path(ir)
}

#[test]
fn import_path_stem_exact_with_appended_extension() {
    // `partial` from `views/page.tmpl` → `views/partial.tmpl`, bound by stem.
    let lookup = Lookup::new().with_in_file(
        "views/partial.tmpl",
        sym(
            42,
            "partial",
            "views/partial.tmpl::partial",
            "class",
            "views/partial.tmpl",
        ),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("partial");
    let res = run_import_path(&lookup, &fc, &r, &base_ir()).expect("stem-exact resolves");
    assert_eq!(res.target_symbol_id, 42);
    assert_eq!(res.strategy, "test_template_include");
    assert_eq!(res.confidence, 1.0);
}

#[test]
fn import_path_stem_exact_rejects_wrong_kind() {
    // The only candidate symbol is a `function`, not the configured `class`.
    let lookup = Lookup::new().with_in_file(
        "views/partial.tmpl",
        sym(1, "partial", "q", "function", "views/partial.tmpl"),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("partial");
    assert!(run_import_path(&lookup, &fc, &r, &base_ir()).is_none());
}

#[test]
fn import_path_target_with_extension_taken_verbatim() {
    // Target already carries a known extension — taken as-is, no appended forms.
    let lookup = Lookup::new().with_in_file(
        "views/partial.tmpl",
        sym(7, "partial", "q", "class", "views/partial.tmpl"),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("partial.tmpl");
    let res = run_import_path(&lookup, &fc, &r, &base_ir()).expect("verbatim ext resolves");
    assert_eq!(res.target_symbol_id, 7);
}

#[test]
fn import_path_underscore_stripped_match() {
    // `_card.tmpl` partial; the indexed symbol is named `card` (no underscore).
    let lookup = Lookup::new().with_in_file(
        "views/_card.tmpl",
        sym(9, "card", "q", "class", "views/_card.tmpl"),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("card");
    let mut ir = base_ir();
    ir.underscore_variant = true;
    ir.stem_match = StemMatch::StemOrUnderscoreStripped;
    let res = run_import_path(&lookup, &fc, &r, &ir).expect("underscore sibling resolves");
    assert_eq!(res.target_symbol_id, 9);
}

#[test]
fn import_path_basename_with_ext_match() {
    // GitHub-Actions style: the binding symbol's name is the full basename.
    let lookup = Lookup::new().with_in_file(
        "ci/build/action.yml",
        sym(13, "action.yml", "q", "class", "ci/build/action.yml"),
    );
    let fc = file_ctx_at("ci/workflow.yml");
    let r = extracted_import("build");
    let mut ir = base_ir();
    ir.extensions = &["yml", "yaml"];
    ir.index_files = &["action"];
    ir.stem_match = StemMatch::BasenameWithExt;
    let res = run_import_path(&lookup, &fc, &r, &ir).expect("action.yml index resolves");
    assert_eq!(res.target_symbol_id, 13);
}

#[test]
fn import_path_any_class_in_file_match() {
    // GSP: no name check — any class-kind symbol in the candidate file binds.
    // `_x.gsp` partial-file convention via underscore variant.
    let lookup = Lookup::new().with_in_file(
        "views/_x.gsp",
        sym(21, "SomethingElse", "q", "class", "views/_x.gsp"),
    );
    let fc = file_ctx_at("views/show.gsp");
    let r = extracted_import("x");
    let mut ir = base_ir();
    ir.extensions = &["gsp"];
    ir.underscore_variant = true;
    ir.decline_leading_slash = true;
    ir.stem_match = StemMatch::AnyClassInFile;
    let res = run_import_path(&lookup, &fc, &r, &ir).expect("any-class resolves");
    assert_eq!(res.target_symbol_id, 21);
}

#[test]
fn import_path_decline_leading_slash() {
    let lookup =
        Lookup::new().with_in_file("views/_x.gsp", sym(1, "x", "q", "class", "views/_x.gsp"));
    let fc = file_ctx_at("views/show.gsp");
    let r = extracted_import("/shared/x");
    let mut ir = base_ir();
    ir.extensions = &["gsp"];
    ir.decline_leading_slash = true;
    assert!(
        run_import_path(&lookup, &fc, &r, &ir).is_none(),
        "a views-root-relative leading-slash target must be declined"
    );
}

#[test]
fn import_path_index_file_entry() {
    // `pkg` from `views/page.tmpl` → `views/pkg/index.tmpl`, bound by stem.
    let lookup = Lookup::new().with_in_file(
        "views/pkg/index.tmpl",
        sym(31, "index", "q", "class", "views/pkg/index.tmpl"),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("pkg");
    let mut ir = base_ir();
    ir.index_files = &["index"];
    let res = run_import_path(&lookup, &fc, &r, &ir).expect("index entry resolves");
    assert_eq!(res.target_symbol_id, 31);
}

#[test]
fn import_path_kebab_variant() {
    // `UserCard` → kebab `user-card` → `views/user-card.tmpl`.
    let lookup = Lookup::new().with_in_file(
        "views/user-card.tmpl",
        sym(44, "user-card", "q", "class", "views/user-card.tmpl"),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("UserCard");
    let mut ir = base_ir();
    ir.kebab_variant = true;
    let res = run_import_path(&lookup, &fc, &r, &ir).expect("kebab variant resolves");
    assert_eq!(res.target_symbol_id, 44);
}

#[test]
fn import_path_walk_up_partial_dirs() {
    // Handlebars partials walk: `header` from `views/pages/home.tmpl` resolves
    // to `views/partials/header.tmpl` one directory up.
    let lookup = Lookup::new().with_in_file(
        "views/partials/header.tmpl",
        sym(55, "header", "q", "class", "views/partials/header.tmpl"),
    );
    let fc = file_ctx_at("views/pages/home.tmpl");
    let r = extracted_import("header");
    let mut ir = base_ir();
    ir.candidate_dirs = CandidateDirs::WalkUp {
        dirs: &["partials"],
        depth: 4,
    };
    let res = run_import_path(&lookup, &fc, &r, &ir).expect("partial-dir walk resolves");
    assert_eq!(res.target_symbol_id, 55);
}

#[test]
fn import_path_declines_non_imports_ref() {
    // A non-`Imports` ref never enters the template strategy.
    let lookup = Lookup::new().with_in_file(
        "views/partial.tmpl",
        sym(1, "partial", "q", "class", "views/partial.tmpl"),
    );
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_call("partial"); // Calls, not Imports
    assert!(run_import_path(&lookup, &fc, &r, &base_ir()).is_none());
}

#[test]
fn import_path_declines_empty_target() {
    let lookup = Lookup::new();
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("   ");
    assert!(run_import_path(&lookup, &fc, &r, &base_ir()).is_none());
}

#[test]
fn import_path_no_candidate_returns_none() {
    // No symbol indexed at any candidate path → unresolved.
    let lookup = Lookup::new();
    let fc = file_ctx_at("views/page.tmpl");
    let r = extracted_import("missing");
    assert!(run_import_path(&lookup, &fc, &r, &base_ir()).is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_module_anchor — module-anchored bind off r.module
// ---------------------------------------------------------------------------

use crate::type_checker::profile::language_profile::{
    ExtMatch, ExternalByImport, ModuleAnchor, ModuleAnchorBind, ModulePrefixRewrites,
    RelativeMarker, StemSource,
};

#[test]
fn module_anchor_name_exact_kind_binds_via_in_module_from() {
    // Python relative import / Dart library prefix: the module resolves to a
    // project file; the bare name binds there by name + kind.
    let lookup = Lookup::new()
        .with_module_file("./helpers", "myapp/services/helpers.py")
        .with_in_file(
            "myapp/services/helpers.py",
            sym(
                7,
                "do_work",
                "helpers.do_work",
                "function",
                "myapp/services/helpers.py",
            ),
        );
    let r = extracted_call_with_module("do_work", "./helpers");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::NameExactKind),
            RelativeMarker::DotSlashPrefix,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("name-exact anchor resolves");
    assert_eq!(resolved.target_symbol_id, 7);
    assert_eq!(resolved.strategy, "default_module_anchor");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn module_anchor_off_is_inert() {
    let lookup = Lookup::new()
        .with_module_file("./helpers", "h.py")
        .with_in_file("h.py", sym(7, "do_work", "do_work", "function", "h.py"));
    let r = extracted_call_with_module("do_work", "./helpers");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_module_anchor(
            ModuleAnchor::Off,
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .is_none(),
        "Off leaves the anchor inert"
    );
}

#[test]
fn module_anchor_returns_none_without_module() {
    let lookup = Lookup::new();
    let r = extracted_call("do_work"); // no module
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::NameExactKind),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .is_none());
}

#[test]
fn module_anchor_prefer_named_else_first_picks_same_named() {
    // Ruby `require "sidekiq/api"` → Sidekiq::Api: the same-named module symbol
    // (case-insensitive) wins over an unrelated sibling in the same file.
    let lookup = Lookup::new()
        .with_module_file("sidekiq/api", "vendor/sidekiq/api.rb")
        .with_in_file(
            "vendor/sidekiq/api.rb",
            sym(1, "Helper", "Helper", "method", "vendor/sidekiq/api.rb"),
        )
        .with_in_file(
            "vendor/sidekiq/api.rb",
            sym(
                2,
                "Api",
                "Sidekiq.Api",
                "namespace",
                "vendor/sidekiq/api.rb",
            ),
        );
    let r = {
        let mut r = extracted_call_with_module("api", "sidekiq/api");
        r.kind = EdgeKind::Imports;
        r
    };
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::PreferNamedElseFirst),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("require anchor resolves");
    assert_eq!(resolved.target_symbol_id, 2, "same-named symbol preferred");
}

#[test]
fn module_anchor_prefer_named_else_first_falls_back_to_first() {
    // No same-named symbol → the first symbol in the file anchors the edge.
    let lookup = Lookup::new()
        .with_module_file("logger", "vendor/logger.rb")
        .with_in_file(
            "vendor/logger.rb",
            sym(5, "Setup", "Setup", "method", "vendor/logger.rb"),
        );
    let r = {
        let mut r = extracted_call_with_module("logger", "logger");
        r.kind = EdgeKind::Imports;
        r
    };
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::PreferNamedElseFirst),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("first-symbol anchor resolves");
    assert_eq!(resolved.target_symbol_id, 5);
}

#[test]
fn module_anchor_by_name_under_module_dir_for_absolute_python() {
    // Python `models.TextChoices` — module `models` is absolute (no dot
    // prefix), so the anchor maps it to a directory and binds the bare name in
    // a file under that directory, even though the qname is unqualified.
    let lookup = Lookup::new().with(sym(
        9,
        "TextChoices",
        "TextChoices",
        "class",
        "django/db/models/enums.py",
    ));
    let r = extracted_call_with_module("TextChoices", "models");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::NameExactKind),
            RelativeMarker::DotPrefix,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("dir-containment anchor resolves the absolute module");
    assert_eq!(resolved.target_symbol_id, 9);
}

#[test]
fn module_anchor_by_name_under_module_dir_qname_probe() {
    // The `{module}.{target}` qname probe — `models.User` keyed directly.
    let lookup = Lookup::new().with(sym(10, "User", "models.User", "class", "app/models.py"));
    let r = extracted_call_with_module("User", "models");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("qname probe resolves");
    assert_eq!(resolved.target_symbol_id, 10);
}

#[test]
fn module_anchor_by_name_under_module_dir_colon_module_leaf_fallback() {
    // Rust `use crate::db::DbPool; DbPool::new()` — the call ref carries the
    // verbatim `::` module `crate::db`. The full path fragment `crate/db`
    // doesn't match `src/db.rs`, so the bind falls back to the module leaf
    // `db`, whose stem matches the file basename.
    let lookup = Lookup::new().with(sym(11, "new", "DbPool.new", "method", "src/db.rs"));
    let r = extracted_call_with_module("new", "crate::db");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
            RelativeMarker::None,
            NameNormalization::None,
            "::",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("`::` module leaf fallback resolves");
    assert_eq!(resolved.target_symbol_id, 11);
    assert_eq!(resolved.strategy, "default_module_anchor");
}

#[test]
fn module_anchor_by_name_under_module_dir_colon_qname_probe() {
    // When a `::`-keyed qname IS in the index (`{module}::{target}`), the probe
    // matches it directly — the `::` separator is tried alongside the universal
    // `.` join.
    let lookup = Lookup::new().with(sym(
        12,
        "read",
        "lemmy::source::read",
        "function",
        "ext/lemmy.rs",
    ));
    let r = extracted_call_with_module("read", "lemmy::source");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
            RelativeMarker::None,
            NameNormalization::None,
            "::",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("`::` qname probe resolves");
    assert_eq!(resolved.target_symbol_id, 12);
}

#[test]
fn module_anchor_by_file_stem_binds_on_basename_stem() {
    // OCaml `List.map` — module leaf `list` (lowercased) matches the basename
    // stem of `list.ml`, where `map` is declared.
    let lookup = Lookup::new().with(sym(50, "map", "List.map", "function", "lib/list.ml"));
    let r = extracted_call_with_module("map", "List");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByFileStem {
                against: StemSource::ModuleLeaf,
            }),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("file-stem anchor resolves on basename stem");
    assert_eq!(resolved.target_symbol_id, 50);
    assert_eq!(resolved.strategy, "default_module_anchor");
}

#[test]
fn module_anchor_by_file_stem_strips_dotted_module_head() {
    // A dotted module `Stdlib.List` — the head `Stdlib` is an alias root; the
    // LEAF `list` drives the stem match against `list.ml`.
    let lookup = Lookup::new().with(sym(51, "map", "List.map", "function", "lib/list.ml"));
    let r = extracted_call_with_module("map", "Stdlib.List");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByFileStem {
                against: StemSource::ModuleLeaf,
            }),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("dotted-module leaf drives the stem match");
    assert_eq!(resolved.target_symbol_id, 51);
}

#[test]
fn module_anchor_by_file_stem_matches_dir_segment() {
    // The leaf matches a path DIR segment (`/list/`), not just a basename.
    let lookup = Lookup::new().with(sym(52, "map", "List.map", "function", "lib/list/core.ml"));
    let r = extracted_call_with_module("map", "List");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByFileStem {
                against: StemSource::ModuleLeaf,
            }),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("dir-segment match resolves");
    assert_eq!(resolved.target_symbol_id, 52);
}

#[test]
fn module_anchor_by_file_stem_declines_unrelated_file() {
    // No candidate whose stem / dir-segment equals the leaf → no bind.
    let lookup = Lookup::new().with(sym(53, "map", "Other.map", "function", "lib/other.ml"));
    let r = extracted_call_with_module("map", "List");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::ByFileStem {
                against: StemSource::ModuleLeaf,
            }),
            RelativeMarker::None,
            NameNormalization::None,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .is_none());
}

#[test]
fn module_anchor_member_of_module_type_folds_case() {
    // Fortran derived-type member: `module` names a TYPE; the member's name is
    // compared under a case-insensitive NormSpec, so `getval` (ref) binds the
    // `GetVal` member declared on type `Particle`.
    let lookup = Lookup::new().with_member(
        "Particle",
        sym(
            60,
            "GetVal",
            "Particle.GetVal",
            "method",
            "src/particle.f90",
        ),
    );
    let r = extracted_call_with_module("getval", "Particle");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let norm = NameNormalization::Spec(NormSpec {
        case_insensitive: true,
        strip_chars: &[],
        strip_prefixes: &[],
        strip_sigils: &[],
    });
    let resolved = d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::MemberOfModuleType),
            RelativeMarker::None,
            norm,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .expect("case-insensitive member of module type resolves");
    assert_eq!(resolved.target_symbol_id, 60);
    assert_eq!(resolved.strategy, "default_module_anchor");
}

#[test]
fn module_anchor_member_of_module_type_declines_unknown_member() {
    // No member of the type matches the target → no bind.
    let lookup = Lookup::new().with_member(
        "Particle",
        sym(
            61,
            "GetVal",
            "Particle.GetVal",
            "method",
            "src/particle.f90",
        ),
    );
    let r = extracted_call_with_module("missing", "Particle");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let norm = NameNormalization::Spec(NormSpec {
        case_insensitive: true,
        strip_chars: &[],
        strip_prefixes: &[],
        strip_sigils: &[],
    });
    assert!(d
        .resolve_via_module_anchor(
            ModuleAnchor::On(ModuleAnchorBind::MemberOfModuleType),
            RelativeMarker::None,
            norm,
            ".",
            ModulePrefixRewrites::Off,
            false,
            &accept_any,
        )
        .is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_same_dir — bare target in a sibling file of the same directory
// ---------------------------------------------------------------------------

#[test]
fn same_dir_binds_sibling_in_same_directory() {
    // Odin: the source file's parent dir `game` is the package; a bare target
    // declared in a sibling file of the same dir resolves.
    let lookup = Lookup::new().with(sym(
        70,
        "update",
        "update",
        "function",
        "src/game/player.odin",
    ));
    let r = extracted_call("update");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/game/world.odin".to_string(),
        language: "odin".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_same_dir(&accept_any)
        .expect("same-dir sibling resolves");
    assert_eq!(resolved.target_symbol_id, 70);
    assert_eq!(resolved.strategy, "default_same_dir");
}

#[test]
fn same_dir_declines_candidate_in_other_directory() {
    // A candidate in a DIFFERENT directory is a different package → no bind.
    let lookup = Lookup::new().with(sym(
        71,
        "update",
        "update",
        "function",
        "src/render/gpu.odin",
    ));
    let r = extracted_call("update");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/game/world.odin".to_string(),
        language: "odin".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_same_dir(&accept_any).is_none(),
        "cross-directory candidate is a different package"
    );
}

// ---------------------------------------------------------------------------
// End-to-end through resolve_all_with_profile — OCaml / Fortran / Odin
// ---------------------------------------------------------------------------

#[test]
fn ocaml_profile_resolves_dotted_module_via_file_stem() {
    let lookup = Lookup::new().with(sym(80, "map", "List.map", "function", "lib/list.ml"));
    let r = extracted_call_with_module("map", "List");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "lib/main.ml".to_string(),
        language: "ocaml".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::ocaml::OCAML_PROFILE)
        .expect("ocaml file-stem anchor resolves through the ladder");
    assert_eq!(resolved.target_symbol_id, 80);
    assert_eq!(resolved.strategy, "default_module_anchor");
}

/// `open Mylib` injects every top-level binding of `mylib.ml` into bare scope.
/// A top-level `let helper` carries qname `helper` (no module prefix), so the
/// candidate's FILE-stem — not its qname — names the open'd module; the
/// FileStem wildcard rung binds the bare `helper` call.
#[test]
fn ocaml_open_injects_module_members_into_bare_scope() {
    let lookup = Lookup::new().with(sym(81, "helper", "helper", "function", "lib/mylib.ml"));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("Mylib")], None);
    fc.language = "ocaml".to_string();
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_all_with_profile(&crate::languages::ocaml::OCAML_PROFILE)
    .expect("open'd module member binds via file-stem wildcard");
    assert_eq!(resolved.target_symbol_id, 81);
    assert_eq!(resolved.strategy, "default_wildcard_import");
}

/// A bare name that lives in no open'd module declines — the open'd file is the
/// gate, so `helper` in `other.ml` is unreachable through `open Mylib`.
#[test]
fn ocaml_bare_name_outside_open_declines() {
    let lookup = Lookup::new().with(sym(82, "helper", "helper", "function", "lib/other.ml"));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("Mylib")], None);
    fc.language = "ocaml".to_string();
    assert!(
        (DefaultResolver {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind_compatible: accept_any,
        })
        .resolve_all_with_profile(&crate::languages::ocaml::OCAML_PROFILE)
        .is_none(),
        "a bare name in an un-open'd module must not bind"
    );
}

/// Qualified `Mylib.helper` still binds through `module_anchor` (ByFileStem),
/// untouched by the wildcard flip — the wildcard rung is skipped for dotted
/// targets, so qualified resolution stays on the anchor path.
#[test]
fn ocaml_qualified_member_still_binds_via_module_anchor() {
    let lookup = Lookup::new().with(sym(83, "helper", "helper", "function", "lib/mylib.ml"));
    let r = extracted_call_with_module("helper", "Mylib");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![], None);
    fc.language = "ocaml".to_string();
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_all_with_profile(&crate::languages::ocaml::OCAML_PROFILE)
    .expect("qualified Mylib.helper resolves via module anchor");
    assert_eq!(resolved.target_symbol_id, 83);
    assert_eq!(resolved.strategy, "default_module_anchor");
}

#[test]
fn fortran_profile_resolves_derived_type_member_case_insensitively() {
    let lookup = Lookup::new().with_member(
        "Particle",
        sym(
            81,
            "GetVal",
            "Particle.GetVal",
            "method",
            "src/particle.f90",
        ),
    );
    let r = extracted_call_with_module("getval", "Particle");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/main.f90".to_string(),
        language: "fortran".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::fortran::FORTRAN_PROFILE)
        .expect("fortran derived-type member resolves through the ladder");
    assert_eq!(resolved.target_symbol_id, 81);
    assert_eq!(resolved.strategy, "default_module_anchor");
}

#[test]
fn fortran_profile_folds_case_in_same_file_step() {
    // With name_normalization set, the same-file step also folds case — a
    // module-less ref binds a differently-cased sibling without a pre-resolver.
    let lookup = Lookup::new().with_in_file(
        "src/main.f90",
        sym(82, "Compute", "Compute", "function", "src/main.f90"),
    );
    let r = extracted_call("compute");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/main.f90".to_string(),
        language: "fortran".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::fortran::FORTRAN_PROFILE)
        .expect("case-folded same-file sibling resolves");
    assert_eq!(resolved.target_symbol_id, 82);
    assert_eq!(resolved.strategy, "default_same_file");
}

#[test]
fn odin_profile_resolves_same_package_directory_sibling() {
    let lookup = Lookup::new().with(sym(
        83,
        "update",
        "update",
        "function",
        "src/game/player.odin",
    ));
    let r = extracted_call("update");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/game/world.odin".to_string(),
        language: "odin".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::odin::ODIN_PROFILE)
        .expect("odin same-package directory sibling resolves through the ladder");
    assert_eq!(resolved.target_symbol_id, 83);
    assert_eq!(resolved.strategy, "default_same_dir");
}

#[test]
fn odin_module_scope_same_dir_unchanged() {
    // Odin's ModuleScope::SameDir binds a same-parent-dir sibling through the
    // module-scope rung with the same-dir strategy tag — the SameDir arm
    // delegates to resolve_via_same_dir.
    let lookup = Lookup::new().with(sym(84, "spawn", "spawn", "function", "src/game/enemy.odin"));
    let r = extracted_call("spawn");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/game/world.odin".to_string(),
        language: "odin".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::odin::ODIN_PROFILE)
        .expect("odin SameDir module-scope still binds the same-dir sibling");
    assert_eq!(resolved.target_symbol_id, 84);
    assert_eq!(resolved.strategy, "default_same_dir");
}

#[test]
fn run_ladder_terminal_declines_local_homonym_on_anchor_miss() {
    // Dart BindThenDecline: a prefixed (non-Imports) ref whose module misses
    // must NOT bind to a same-named local symbol — the ladder terminates.
    let lookup = Lookup::new()
        .with(sym(20, "Value", "Value", "class", "src/local.dart"))
        .with_in_file(
            "src/local.dart",
            sym(20, "Value", "Value", "class", "src/local.dart"),
        );
    // module is set but resolves to no project file (external library prefix).
    // TypeRef so a `class` candidate is kind-compatible — the terminal guard,
    // not the kind table, is what must decline the local homonym.
    let r = {
        let mut r = extracted_call_with_module("Value", "package:drift/drift.dart");
        r.kind = EdgeKind::TypeRef;
        r
    };
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "src/local.dart".to_string(),
        language: "dart".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_all_with_profile(&crate::languages::dart::DART_PROFILE)
            .is_none(),
        "terminal guard declines so the external prefix isn't hijacked"
    );
}

#[test]
fn run_ladder_non_terminal_falls_through_on_anchor_miss() {
    // Python is non-terminal: a module-carrying ref whose anchor misses still
    // falls through to the regular ladder (here, same-file sibling).
    let lookup = Lookup::new().with_in_file(
        "app/views.py",
        sym(30, "helper", "helper", "function", "app/views.py"),
    );
    // Absolute module that maps to no indexed dir → anchor miss, then fall
    // through to same-file.
    let r = extracted_call_with_module("helper", "unrelated_pkg");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "app/views.py".to_string(),
        language: "python".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::python::PYTHON_PROFILE)
        .expect("non-terminal anchor miss falls through to same-file");
    assert_eq!(resolved.target_symbol_id, 30);
}

// ---------------------------------------------------------------------------
// resolve_via_external_by_import — import-scoped external bind
// ---------------------------------------------------------------------------

#[test]
fn external_by_import_binds_gem_family() {
    // `aws-sdk-s3` external symbol resolves under gem `aws` (the import root)
    // via the `{root}-` family rule.
    let lookup = Lookup::new().with(sym(
        40,
        "Client",
        "Aws.S3.Client",
        "class",
        "ext:ruby:aws-sdk-s3/lib/aws-sdk-s3/client.rb",
    ));
    let r = extracted_call("Client");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![import("aws", Some("aws"))], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_external_by_import(&ExternalByImport, ExtMatch::PkgSegment, &accept_any)
        .expect("gem-family external resolves");
    assert_eq!(resolved.target_symbol_id, 40);
    assert_eq!(resolved.strategy, "default_external_by_import");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn external_by_import_declines_unimported_gem() {
    // The external's gem is not in the file's import set → no bind.
    let lookup = Lookup::new().with(sym(
        41,
        "Client",
        "Stripe.Client",
        "class",
        "ext:ruby:stripe/lib/stripe/client.rb",
    ));
    let r = extracted_call("Client");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![import("aws", Some("aws"))], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_external_by_import(&ExternalByImport, ExtMatch::PkgSegment, &accept_any,)
        .is_none());
}

#[test]
fn external_by_import_ignores_internal_symbols() {
    // A same-named INTERNAL symbol must not be picked by the external strategy.
    let lookup = Lookup::new().with(sym(42, "Client", "app.Client", "class", "app/client.rb"));
    let r = extracted_call("Client");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![import("app", Some("app"))], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_external_by_import(&ExternalByImport, ExtMatch::PkgSegment, &accept_any,)
        .is_none());
}

// ---------------------------------------------------------------------------
// self-keyword strip — reuses profile.self_keywords in scope / same-file
// ---------------------------------------------------------------------------

#[test]
fn scope_visible_strips_leading_self_keyword() {
    // `self.method` → strip `self.`, probe `{scope}.method`.
    let lookup = Lookup::new().with(sym(50, "method", "MyClass.method", "function", "app/m.py"));
    let r = extracted_call("self.method");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec!["MyClass".to_string()]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_scope_visible(
            &accept_any,
            &["."],
            &["self", "cls"],
            NameNormalization::None,
        )
        .expect("self.-stripped scope probe resolves");
    assert_eq!(resolved.target_symbol_id, 50);
}

#[test]
fn scope_visible_empty_self_keywords_does_not_strip() {
    // With no self keywords (default), `self.method` is probed verbatim and
    // does not match the bare `method` member — byte-identical to before.
    let lookup = Lookup::new().with(sym(51, "method", "MyClass.method", "function", "app/m.py"));
    let r = extracted_call("self.method");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec!["MyClass".to_string()]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_scope_visible(&accept_any, &["."], &[], NameNormalization::None)
            .is_none(),
        "no strip with empty self_keywords"
    );
}

// ---------------------------------------------------------------------------
// module_skip — module-keyed pre-ladder decline (sibling of builtin_skip)
// ---------------------------------------------------------------------------

use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};

/// Profile mirroring DEFAULT but declining any ref whose `module` is one of two
/// non-project providers — the shape SCSS folds into (`__css_fn__` synthesized
/// hint OR a `sass:`-prefixed built-in module).
static MODULE_SKIP_PROFILE: LanguageProfile = LanguageProfile {
    module_skip: Some(|m| m == "__css_fn__" || m.starts_with("sass:")),
    ..DEFAULT_PROFILE
};

/// Resolve through the full profile ladder against a single same-file sibling
/// named `target`, with the ref carrying `module`. Absent any decline the
/// same-file strategy binds the sibling (id 1).
fn resolve_module_sibling(
    profile: &LanguageProfile,
    target: &str,
    module: &str,
) -> Option<Resolution> {
    let sibling = sym(1, target, target, "function", "src/main.ts");
    let lookup = Lookup::new().with_in_file("src/main.ts", sibling);
    let r = extracted_call_with_module(target, module);
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    d.resolve_all_with_profile(profile)
}

#[test]
fn module_skip_declines_before_ladder_binds_sibling() {
    // With no module_skip, the same-file strategy binds the sibling even when
    // the ref carries a module (DEFAULT_PROFILE has module_anchor Off, so a
    // module-carrying ref falls straight through to the bare-name strategies).
    let bound = resolve_module_sibling(&DEFAULT_PROFILE, "lighten", "sass:color")
        .expect("the ladder binds the same-file sibling when nothing skips the module");
    assert_eq!(bound.target_symbol_id, 1);

    // With module_skip recognizing `sass:`-prefixed modules, the ladder declines
    // before any strategy — the sibling is NOT bound, leaving the ref for
    // external classification.
    assert!(
        resolve_module_sibling(&MODULE_SKIP_PROFILE, "lighten", "sass:color").is_none(),
        "module_skip must decline the ref before the ladder binds a homonym"
    );
}

#[test]
fn module_skip_declines_synthesized_hint_module() {
    // The second decline branch: a synthesized non-project hint module declines
    // identically, proving the predicate (not a single literal) drives the gate.
    assert!(
        resolve_module_sibling(&MODULE_SKIP_PROFILE, "rgba", "__css_fn__").is_none(),
        "module_skip declines the synthesized hint module"
    );
}

#[test]
fn module_skip_leaves_other_modules_resolvable() {
    // A ref whose module the predicate does NOT match still resolves through the
    // ladder under the same profile — the gate is per-module, not a blanket
    // decline of every module-carrying ref.
    let bound = resolve_module_sibling(&MODULE_SKIP_PROFILE, "render", "./_card")
        .expect("a non-skipped module still resolves the same-file sibling");
    assert_eq!(bound.target_symbol_id, 1);
}

#[test]
fn module_skip_ignores_refs_without_a_module() {
    // module_skip keys on the ref's `module`; a moduleless ref is never declined
    // even when its bare target would match the predicate string.
    let sibling = sym(1, "sass:color", "sass:color", "function", "src/main.ts");
    let lookup = Lookup::new().with_in_file("src/main.ts", sibling);
    let r = extracted_call("sass:color"); // no module
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let bound = d
        .resolve_all_with_profile(&MODULE_SKIP_PROFILE)
        .expect("a moduleless ref is unaffected by module_skip");
    assert_eq!(bound.target_symbol_id, 1);
}

#[test]
fn module_skip_none_is_inert() {
    // The default (None) leaves every module-carrying ref on the ladder — a ref
    // whose module would have matched a predicate still binds.
    let bound = resolve_module_sibling(&DEFAULT_PROFILE, "rgba", "__css_fn__")
        .expect("module_skip None resolves the sibling for any module");
    assert_eq!(bound.target_symbol_id, 1);
}

// ---------------------------------------------------------------------------
// name_normalization — normalize_name transform + bare-name comparison
// ---------------------------------------------------------------------------

const CASE_FOLD_SPEC: NormSpec = NormSpec {
    case_insensitive: true,
    strip_chars: &[],
    strip_prefixes: &[],
    strip_sigils: &[],
};

#[test]
fn normalize_name_none_is_identity_and_borrowed() {
    // The default transform must not change a single byte and must not
    // allocate — it returns the input borrowed verbatim.
    let out = normalize_name(NameNormalization::None, "MixedCaseName");
    assert_eq!(out, "MixedCaseName");
    assert!(
        matches!(out, std::borrow::Cow::Borrowed(_)),
        "None must borrow, never allocate"
    );
}

#[test]
fn normalize_name_empty_spec_is_identity_and_borrowed() {
    // A spec whose every delta is off reduces to identity — also borrowed.
    let spec = NormSpec {
        case_insensitive: false,
        strip_chars: &[],
        strip_prefixes: &[],
        strip_sigils: &[],
    };
    let out = normalize_name(NameNormalization::Spec(spec), "MixedCaseName");
    assert_eq!(out, "MixedCaseName");
    assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn normalize_name_case_insensitive_folds() {
    let out = normalize_name(NameNormalization::Spec(CASE_FOLD_SPEC), "MyProc");
    assert_eq!(out, "myproc");
    assert!(matches!(out, std::borrow::Cow::Owned(_)));
}

#[test]
fn normalize_name_strips_sigil_pair_then_prefix_then_chars() {
    // A spec exercising every delta in its fixed order: a `${...}` sigil
    // wrapper, a leading `@` prefix, removed `_` chars, then case fold.
    let spec = NormSpec {
        case_insensitive: true,
        strip_chars: &['_'],
        strip_prefixes: &["@"],
        strip_sigils: &[("${", "}")],
    };
    // `${@My_Var}` → strip sigil → `@My_Var` → strip `@` → `My_Var`
    //   → strip `_` → `MyVar` → fold → `myvar`.
    let out = normalize_name(NameNormalization::Spec(spec), "${@My_Var}");
    assert_eq!(out, "myvar");
}

#[test]
fn normalize_name_strips_prefix_case_insensitively_when_folding() {
    // A case-insensitive spec strips a declared prefix regardless of the call
    // site's casing — a lowercase `when ` must strip against a title-case
    // `When ` prefix (the BDD-prefix shape). The strip runs before the
    // case-fold step, so without case-insensitive prefix matching a lowercase
    // call would slip through unstripped.
    let spec = NormSpec {
        case_insensitive: true,
        strip_chars: &[' '],
        strip_prefixes: &["When "],
        strip_sigils: &[],
    };
    // `when I Park` → strip `When ` (folded) → `I Park` → strip ` ` → fold → `ipark`.
    assert_eq!(
        normalize_name(NameNormalization::Spec(spec), "when I Park"),
        "ipark"
    );
    // Title-case form folds to the same normalized key.
    assert_eq!(
        normalize_name(NameNormalization::Spec(spec), "When I Park"),
        "ipark"
    );
    // No prefix → unchanged but for space-strip + fold.
    assert_eq!(
        normalize_name(NameNormalization::Spec(spec), "I Park"),
        "ipark"
    );
}

#[test]
fn normalize_name_case_sensitive_prefix_stays_exact() {
    // Without case folding, the prefix match stays byte-exact: a differently
    // cased prefix does NOT strip.
    let spec = NormSpec {
        case_insensitive: false,
        strip_chars: &[],
        strip_prefixes: &["When "],
        strip_sigils: &[],
    };
    assert_eq!(
        normalize_name(NameNormalization::Spec(spec), "when foo"),
        "when foo"
    );
    assert_eq!(
        normalize_name(NameNormalization::Spec(spec), "When foo"),
        "foo"
    );
}

#[test]
fn normalize_name_bare_sigil_prefix_strips() {
    // A `(prefix, "")` pair strips a bare leading sigil with no closing form.
    let spec = NormSpec {
        case_insensitive: false,
        strip_chars: &[],
        strip_prefixes: &[],
        strip_sigils: &[("$", "")],
    };
    let out = normalize_name(NameNormalization::Spec(spec), "$count");
    assert_eq!(out, "count");
}

#[test]
fn same_file_none_normalization_is_byte_identical() {
    // The byte-identity guarantee: with NameNormalization::None a sibling whose
    // name differs only by case does NOT bind — the comparison is byte-for-byte,
    // exactly the pre-axis behavior for every case-sensitive language.
    let sibling = sym(120, "Helper", "Helper", "function", "src/main.ts");
    let lookup = Lookup::new().with_in_file("src/main.ts", sibling);
    let r = extracted_call("helper"); // lower-case ref, capitalized sibling
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_same_file(&accept_any, &[], NameNormalization::None)
            .is_none(),
        "a non-case-fold lookup must not bind a different-case sibling"
    );
}

#[test]
fn same_file_case_insensitive_binds_different_case_sibling() {
    // The opt-in case: a case-insensitive spec binds the same different-case
    // sibling the None lookup left unresolved.
    let sibling = sym(121, "Helper", "Helper", "function", "src/main.ts");
    let lookup = Lookup::new().with_in_file("src/main.ts", sibling);
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_same_file(&accept_any, &[], NameNormalization::Spec(CASE_FOLD_SPEC))
        .expect("case-insensitive spec binds the different-case sibling");
    assert_eq!(resolved.target_symbol_id, 121);
    assert_eq!(resolved.strategy, "default_same_file");
}

#[test]
fn scope_visible_none_normalization_is_byte_identical() {
    // The exact qname probe is the whole strategy under None: a `{scope}.{target}`
    // built from a lower-case target cannot match a `Scope.Helper`-keyed member,
    // and no normalized fallback runs — byte-identical to before the axis.
    let lookup = Lookup::new().with(sym(
        130,
        "Helper",
        "Scope.Helper",
        "function",
        "src/main.rs",
    ));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["Scope".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_scope_visible(&accept_any, &["."], &[], NameNormalization::None)
            .is_none(),
        "the exact qname probe cannot fold case; None runs no fallback"
    );
}

#[test]
fn scope_visible_case_insensitive_binds_scope_member_via_fallback() {
    // With a case-insensitive spec the exact qname probe still misses (the index
    // is case-sensitive) but the normalized members_of fallback binds the member.
    let member = sym(131, "Helper", "Scope.Helper", "function", "src/main.rs");
    let lookup = Lookup::new()
        .with(member.clone())
        .with_member("Scope", member);
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["Scope".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_scope_visible(
            &accept_any,
            &["."],
            &[],
            NameNormalization::Spec(CASE_FOLD_SPEC),
        )
        .expect("case-insensitive spec binds the scope member via the fallback");
    assert_eq!(resolved.target_symbol_id, 131);
    assert_eq!(resolved.strategy, "default_scope_visible");
}

#[test]
fn scope_visible_exact_qname_probe_still_runs_under_spec() {
    // A spec must not regress the exact-qname path: when the target's surface
    // form already matches the keyed qname, the byte-exact probe binds it before
    // the fallback, at the same id.
    let lookup = Lookup::new().with(sym(
        132,
        "helper",
        "Scope.helper",
        "function",
        "src/main.rs",
    ));
    let r = extracted_call("helper");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec!["Scope".to_string()]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_scope_visible(
            &accept_any,
            &["."],
            &[],
            NameNormalization::Spec(CASE_FOLD_SPEC),
        )
        .expect("exact qname probe binds even under a spec");
    assert_eq!(resolved.target_symbol_id, 132);
}

#[test]
fn is_identity_spec_recognizes_all_default_fields() {
    let identity = NormSpec {
        case_insensitive: false,
        strip_chars: &[],
        strip_prefixes: &[],
        strip_sigils: &[],
    };
    assert!(is_identity_spec(&identity));
    assert!(!is_identity_spec(&CASE_FOLD_SPEC));
}

// ---------------------------------------------------------------------------
// name_normalization threaded into the dotted/qname rungs (qname_exact,
// same_namespace, imported_namespace, namespace_import)
// ---------------------------------------------------------------------------

#[test]
fn qname_exact_folds_case_when_normspec_ci() {
    // A dotted target whose PREFIX case differs from the keyed qname binds under
    // a case-insensitive spec and declines under None. The leaf (`by_name` key)
    // keeps its declared case; the prefix segments fold.
    let lookup = Lookup::new().with(sym(
        140,
        "Do_Thing",
        "Pkg.Sub.Do_Thing",
        "function",
        "src/pkg-sub.adb",
    ));
    let r = extracted_call("pkg.sub.Do_Thing"); // lowercased prefix, declared leaf case
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_qname_exact(false, &accept_any, NameNormalization::Spec(CASE_FOLD_SPEC))
        .expect("case-insensitive spec binds the different-case dotted qname");
    assert_eq!(resolved.target_symbol_id, 140);
    assert_eq!(resolved.strategy, "default_qname_exact");

    // Byte-identity guard: None must NOT bind the differently-cased qname.
    assert!(
        d.resolve_via_qname_exact(false, &accept_any, NameNormalization::None)
            .is_none(),
        "None keeps qname_exact byte-exact for case-sensitive languages"
    );
}

#[test]
fn same_namespace_and_imported_namespace_fold_case_when_ci() {
    // `Alr.Commands.Run` keyed; the file's namespace is `Alr.Commands`; the ref
    // target `run` (lowercase) resolves via same_namespace under the CI spec and
    // declines under None.
    let member = sym(
        141,
        "Run",
        "Alr.Commands.Run",
        "function",
        "src/alr-commands.adb",
    );
    let lookup = Lookup::new().with(member);
    let r = extracted_call("run"); // lowercase, member is `Run`
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], Some("Alr.Commands"));
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_same_namespace(&accept_any, NameNormalization::Spec(CASE_FOLD_SPEC))
        .expect("case-insensitive spec binds the different-case same-namespace member");
    assert_eq!(resolved.target_symbol_id, 141);
    assert_eq!(resolved.strategy, "default_same_namespace");

    assert!(
        d.resolve_via_same_namespace(&accept_any, NameNormalization::None)
            .is_none(),
        "None keeps same_namespace byte-exact"
    );

    // imported_namespace: same candidate reached through an import of the
    // enclosing package rather than the file's own namespace.
    let imp_fc = file_ctx(vec![import("Alr.Commands", Some("Alr.Commands"))], None);
    let imp_rc = ref_ctx(&r, &s, vec![]);
    let imp_d = DefaultResolver {
        file_ctx: &imp_fc,
        ref_ctx: &imp_rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let imp_resolved = imp_d
        .resolve_via_imported_namespace(&accept_any, NameNormalization::Spec(CASE_FOLD_SPEC))
        .expect("case-insensitive spec binds the different-case imported-namespace member");
    assert_eq!(imp_resolved.target_symbol_id, 141);
    assert_eq!(imp_resolved.strategy, "default_imported_namespace");

    assert!(
        imp_d
            .resolve_via_imported_namespace(&accept_any, NameNormalization::None)
            .is_none(),
        "None keeps imported_namespace byte-exact"
    );
}

#[test]
fn namespace_import_folds_case_when_ci() {
    // `using Alr.Commands;` then `Run` — the dotted-prefix import forms
    // `Alr.Commands.run` for a lowercase target and must fold to the keyed
    // `Alr.Commands.Run` under a CI spec, declining under None.
    let lookup = Lookup::new().with(sym(
        142,
        "Run",
        "Alr.Commands.Run",
        "function",
        "src/alr-commands.adb",
    ));
    let r = extracted_call("run");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Alr.Commands", Some("Alr.Commands"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_namespace_import(&accept_any, NameNormalization::Spec(CASE_FOLD_SPEC))
        .expect("case-insensitive spec binds the different-case namespace-import member");
    assert_eq!(resolved.target_symbol_id, 142);
    assert_eq!(resolved.strategy, "default_namespace_import");

    assert!(
        d.resolve_via_namespace_import(&accept_any, NameNormalization::None)
            .is_none(),
        "None keeps namespace_import byte-exact"
    );
}

// ---------------------------------------------------------------------------
// WildcardMatch::FileStem — wildcard import binds by candidate FILE-stem
// ---------------------------------------------------------------------------

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: module.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

/// FileStem mode binds a symbol whose file basename-stem equals the wildcard's
/// module name, with the name comparison folded by the profile's
/// NameNormalization (Pascal is case-insensitive: a `FreeAndNil` symbol binds a
/// `freeandnil` ref because both fold equal AND `by_name` returns it).
#[test]
fn wildcard_file_stem_binds_on_basename_stem() {
    let lookup = Lookup::new().with(sym(
        7,
        "FreeAndNil",
        "FreeAndNil",
        "function",
        "rtl/sysutils.pas",
    ));
    let r = extracted_call("FreeAndNil");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("SysUtils")], None);
    fc.language = "pascal".to_string();
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_all_with_profile(&crate::languages::pascal::PASCAL_PROFILE)
    .expect("FileStem wildcard binds via sysutils.pas stem");
    assert_eq!(resolved.target_symbol_id, 7);
    assert_eq!(resolved.strategy, "default_wildcard_import");
}

/// The underscore-prefix probe accepts an include-file sibling whose stem is
/// `{module}_…` — a unit split across `{unit}_part.inc` files.
#[test]
fn wildcard_file_stem_binds_on_underscore_include() {
    let lookup = Lookup::new().with(sym(
        42,
        "CastleNow",
        "CastleNow",
        "function",
        "src/base/castleutils_now.inc",
    ));
    let r = extracted_call("CastleNow");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("CastleUtils")], None);
    fc.language = "pascal".to_string();
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_all_with_profile(&crate::languages::pascal::PASCAL_PROFILE)
    .expect("FileStem wildcard binds via castleutils_ include stem");
    assert_eq!(resolved.target_symbol_id, 42);
}

/// FileStem mode does not bind when the symbol's file names a different unit
/// than any wildcard import — the unit's file is the gate, not the bare name.
#[test]
fn wildcard_file_stem_declines_unrelated_unit() {
    let lookup = Lookup::new().with(sym(
        42,
        "CastleNow",
        "CastleNow",
        "function",
        "src/base/castleutils_now.inc",
    ));
    let r = extracted_call("CastleNow");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("Classes")], None);
    fc.language = "pascal".to_string();
    assert!(
        (DefaultResolver {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind_compatible: accept_any,
        })
        .resolve_all_with_profile(&crate::languages::pascal::PASCAL_PROFILE)
        .is_none(),
        "a wildcard for an unrelated unit must not bind the symbol"
    );
}

// ---------------------------------------------------------------------------
// JVM static-wildcard test-framework globals (junit/scalatest/mockk) — a bare
// `assertTrue` brought into scope by `import static …Assertions.*` binds to the
// hydrated Maven framework method. Characterizes the generic-ladder bind so a
// future reorder can't silently drop JVM ambient test globals.
// ---------------------------------------------------------------------------

/// `import static org.junit.jupiter.api.Assertions.*;` records the CLASS as the
/// wildcard's module (`module=org.junit.jupiter.api.Assertions`, target=`*`).
/// A bare `assertTrue` ref then binds to the hydrated framework method whose
/// qname is `org.junit.jupiter.api.Assertions.assertTrue` — its qname sits one
/// segment under the imported namespace. The bind flows through the single
/// generic ladder (`resolve_via_imported_namespace` fires first because the
/// static import's module IS the class; `resolve_via_wildcard_import`/QnameUnder
/// is the equivalent fallback for the type-wildcard shape). Both are sound
/// generic rungs, so this asserts the SYMBOL id, not the strategy literal.
#[test]
fn jvm_static_wildcard_import_binds_hydrated_assertion() {
    let lookup = Lookup::new().with(sym(
        9,
        "assertTrue",
        "org.junit.jupiter.api.Assertions.assertTrue",
        "function",
        "ext:java:org.junit.jupiter/junit-jupiter-api/5.10.0/org/junit/jupiter/api/Assertions.java",
    ));
    let r = extracted_call("assertTrue");
    let s = source_symbol("someTestMethod");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(
        vec![wildcard_import("org.junit.jupiter.api.Assertions")],
        None,
    );
    fc.language = "java".to_string();
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_all_with_profile(&crate::languages::java::JAVA_PROFILE)
    .expect("bare static-wildcard-imported assertion binds the hydrated method");
    assert_eq!(resolved.target_symbol_id, 9);
}

/// A wildcard import for a package that does NOT contain the symbol's namespace
/// must decline — the import set is the gate, not the bare name. Locks the
/// widening-only boundary: an `org.assertj.core.api.*` wildcard never pulls a
/// junit `Assertions.assertTrue` into bare scope, because the candidate's qname
/// sits under no imported namespace.
#[test]
fn jvm_wildcard_for_foreign_package_declines() {
    let lookup = Lookup::new().with(sym(
        9,
        "assertTrue",
        "org.junit.jupiter.api.Assertions.assertTrue",
        "function",
        "ext:java:org.junit.jupiter/junit-jupiter-api/5.10.0/org/junit/jupiter/api/Assertions.java",
    ));
    let r = extracted_call("assertTrue");
    let s = source_symbol("someTestMethod");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("org.assertj.core.api")], None);
    fc.language = "java".to_string();
    assert!(
        (DefaultResolver {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind_compatible: accept_any,
        })
        .resolve_all_with_profile(&crate::languages::java::JAVA_PROFILE)
        .is_none(),
        "a wildcard for a foreign package must not bind a symbol under another namespace"
    );
}

// ---------------------------------------------------------------------------
// C# `using static` member-wildcard (the .NET analogue of the JVM static
// wildcard above). `using static System.Math;` brings every static member of
// `System.Math` into bare scope; a bare `Sqrt()` binds to the hydrated NuGet/
// stdlib method whose qname is `System.Math.Sqrt`. C# records the static
// import with `module=System.Math` (the CLASS), so the member's qname sits one
// segment under it — the same generic-ladder shape as junit's `Assertions.*`.
// No per-language code: the bind rides `resolve_via_imported_namespace` /
// `resolve_via_wildcard_import`/QnameUnder, so this pins the SYMBOL id, not a
// strategy literal, and a reorder can't silently drop .NET static members.
// ---------------------------------------------------------------------------

/// `using static System.Math;` + a bare `Sqrt` call binds the hydrated
/// `System.Math.Sqrt` method (qname one segment under the imported module).
#[test]
fn csharp_using_static_member_wildcard_binds_hydrated_method() {
    let lookup = Lookup::new().with(sym(
        14,
        "Sqrt",
        "System.Math.Sqrt",
        "method",
        "ext:dotnet:System.Runtime/System.Private.CoreLib/System/Math.cs",
    ));
    let r = extracted_call("Sqrt");
    let s = source_symbol("Compute");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("System.Math")], None);
    fc.language = "csharp".to_string();
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_all_with_profile(&crate::languages::csharp::CSHARP_PROFILE)
    .expect("bare static-wildcard-imported member binds the hydrated method");
    assert_eq!(resolved.target_symbol_id, 14);
}

/// A `using static` for a foreign type must decline — a `System.Math.Sqrt`
/// candidate stays unbound under a `System.Text` wildcard, because its qname
/// sits under no imported module. Locks the widening-only boundary for .NET.
#[test]
fn csharp_using_static_foreign_type_declines() {
    let lookup = Lookup::new().with(sym(
        14,
        "Sqrt",
        "System.Math.Sqrt",
        "method",
        "ext:dotnet:System.Runtime/System.Private.CoreLib/System/Math.cs",
    ));
    let r = extracted_call("Sqrt");
    let s = source_symbol("Compute");
    let rc = ref_ctx(&r, &s, vec![]);
    let mut fc = file_ctx(vec![wildcard_import("System.Text")], None);
    fc.language = "csharp".to_string();
    assert!(
        (DefaultResolver {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind_compatible: accept_any,
        })
        .resolve_all_with_profile(&crate::languages::csharp::CSHARP_PROFILE)
        .is_none(),
        "a static wildcard for a foreign type must not bind a member under another module"
    );
}

// ---------------------------------------------------------------------------
// ExtMatch::FileStemOrDir — external bind by file-stem / dir against imports
// ---------------------------------------------------------------------------

/// FileStemOrDir matches an external candidate whose basename-stem equals an
/// import LEAF (`httpclient` import → `…/httpclient.nim`).
#[test]
fn ext_match_file_stem_binds_on_import_leaf() {
    let lookup = Lookup::new().with(sym(
        50,
        "getContent",
        "getContent",
        "function",
        "ext:nim:nim-stdlib/pure/httpclient.nim",
    ));
    let r = extracted_call("getContent");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![import("httpclient", Some("std/httpclient"))], None);
    let resolved = (DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    })
    .resolve_via_external_by_import(&ExternalByImport, ExtMatch::FileStemOrDir, &accept_any)
    .expect("external binds via httpclient.nim file stem");
    assert_eq!(resolved.target_symbol_id, 50);
    assert_eq!(resolved.confidence, 1.0);
    assert_eq!(resolved.strategy, "default_external_by_import");
}

/// FileStemOrDir declines when no import leaf / package names the external's
/// file — the import set is the gate.
#[test]
fn ext_match_file_stem_declines_unimported_module() {
    let lookup = Lookup::new().with(sym(
        51,
        "getContent",
        "getContent",
        "function",
        "ext:nim:nim-stdlib/pure/httpclient.nim",
    ));
    let r = extracted_call("getContent");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![import("strutils", Some("std/strutils"))], None);
    assert!(
        (DefaultResolver {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind_compatible: accept_any,
        })
        .resolve_via_external_by_import(&ExternalByImport, ExtMatch::FileStemOrDir, &accept_any,)
        .is_none(),
        "an external not named by any import leaf/package must not bind"
    );
}

// ---------------------------------------------------------------------------
// HeadAliasBind — a dotted target's HEAD names an in-file alias declaration.
// ---------------------------------------------------------------------------

fn wildcard_named_import(name: &str, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

/// A wildcard import whose `imported_name` is the lookup key and whose `alias`
/// carries the decoded bind target (the dynamic-library keyword entry shape).
fn wildcard_alias_import(name: &str, alias: Option<&str>, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: alias.map(|s| s.to_string()),
        is_wildcard: true,
    }
}

const ALIAS_DECODE: AliasDecode = AliasDecode {
    separator: "::",
    fallback_kind: Some("class"),
};

#[test]
fn head_alias_off_is_inert() {
    // The default — a dotted target whose head is a same-file `class` never
    // binds when the strategy is Off.
    let lookup = Lookup::new().with_in_file(
        "src/main.ts",
        sym(60, "google", "google", "class", "src/main.ts"),
    );
    let r = extracted_call("google.compute_instance");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_head_alias(HeadAliasBind::Off, &accept_any)
        .is_none());
}

#[test]
fn head_alias_binds_in_file_head_of_required_kind() {
    let lookup = Lookup::new().with_in_file(
        "src/main.ts",
        sym(61, "google", "google", "class", "src/main.ts"),
    );
    let r = extracted_call("google.compute_instance");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_head_alias(
            HeadAliasBind::OnSameFile {
                require_kind: Some("class"),
            },
            &accept_any,
        )
        .expect("dotted target head binds to the in-file class declaration");
    assert_eq!(resolved.target_symbol_id, 61);
    assert_eq!(resolved.strategy, "default_head_alias");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn head_alias_declines_underscore_head() {
    // A `_`-bearing head is a provider RESOURCE TYPE, not an alias — declined
    // even when a same-file declaration of that name exists.
    let lookup = Lookup::new().with_in_file(
        "src/main.ts",
        sym(62, "aws_instance", "aws_instance", "class", "src/main.ts"),
    );
    let r = extracted_call("aws_instance.web");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_head_alias(
            HeadAliasBind::OnSameFile {
                require_kind: Some("class")
            },
            &accept_any,
        )
        .is_none(),
        "an underscore-bearing head is a resource type, not an alias"
    );
}

#[test]
fn head_alias_declines_bare_target() {
    // No `.` in the target — no head to truncate.
    let lookup = Lookup::new().with_in_file(
        "src/main.ts",
        sym(63, "google", "google", "class", "src/main.ts"),
    );
    let r = extracted_call("google");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_head_alias(
            HeadAliasBind::OnSameFile {
                require_kind: Some("class")
            },
            &accept_any,
        )
        .is_none());
}

#[test]
fn head_alias_require_kind_filters_wrong_kind() {
    // A same-file declaration named `google` exists but is a `function`; the
    // `require_kind: Some("class")` filter rejects it.
    let lookup = Lookup::new().with_in_file(
        "src/main.ts",
        sym(64, "google", "google", "function", "src/main.ts"),
    );
    let r = extracted_call("google.compute_instance");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_head_alias(
            HeadAliasBind::OnSameFile {
                require_kind: Some("class")
            },
            &accept_any,
        )
        .is_none(),
        "require_kind must reject a head of the wrong kind"
    );
}

#[test]
fn head_alias_none_kind_accepts_any() {
    // With `require_kind: None`, any kind-compatible head binds.
    let lookup = Lookup::new().with_in_file(
        "src/main.ts",
        sym(65, "google", "google", "function", "src/main.ts"),
    );
    let r = extracted_call("google.compute_instance");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_head_alias(
            HeadAliasBind::OnSameFile { require_kind: None },
            &accept_any,
        )
        .expect("require_kind None accepts any kind-compatible head");
    assert_eq!(resolved.target_symbol_id, 65);
}

// ---------------------------------------------------------------------------
// FileScopedImports — a bare target binds to a symbol in a file-naming import.
// ---------------------------------------------------------------------------

#[test]
fn file_scoped_import_off_is_inert() {
    let lookup = Lookup::new().with_in_file(
        "common.robot",
        sym(
            70,
            "Open Browser",
            "Open Browser",
            "function",
            "common.robot",
        ),
    );
    let r = extracted_call("Open Browser");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![wildcard_named_import("common", "common.robot")], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_file_scoped_import(
            FileScopedImports::Off,
            NameNormalization::None,
            &accept_any,
        )
        .is_none());
}

#[test]
fn file_scoped_import_binds_symbol_in_imported_file() {
    let lookup = Lookup::new().with_in_file(
        "common.robot",
        sym(
            71,
            "Open Browser",
            "Open Browser",
            "function",
            "common.robot",
        ),
    );
    let r = extracted_call("Open Browser");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![wildcard_named_import("common", "common.robot")], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: None,
            },
            NameNormalization::None,
            &accept_any,
        )
        .expect("bare target binds to a symbol in the imported file");
    assert_eq!(resolved.target_symbol_id, 71);
    assert_eq!(resolved.strategy, "default_file_scoped_import");
}

#[test]
fn file_scoped_import_resolves() {
    let lookup = Lookup::new().with_in_file(
        "lib.py",
        sym(72, "do_thing", "do_thing", "function", "lib.py"),
    );
    let r = extracted_call("do_thing");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![wildcard_named_import("lib", "lib.py")], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: None,
            },
            NameNormalization::None,
            &accept_any,
        )
        .expect("hit");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn file_scoped_import_wildcard_only_skips_non_wildcard() {
    // The same symbol is reachable through a NON-wildcard import; with
    // wildcard_only the import is skipped.
    let lookup = Lookup::new().with_in_file(
        "common.robot",
        sym(
            73,
            "Open Browser",
            "Open Browser",
            "function",
            "common.robot",
        ),
    );
    let r = extracted_call("Open Browser");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("common", Some("common.robot"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: None
            },
            NameNormalization::None,
            &accept_any,
        )
        .is_none(),
        "wildcard_only must skip a non-wildcard import"
    );
}

#[test]
fn file_scoped_import_scans_non_wildcard_when_not_restricted() {
    // wildcard_only: false scans every file-naming import.
    let lookup = Lookup::new().with_in_file(
        "common.robot",
        sym(
            74,
            "Open Browser",
            "Open Browser",
            "function",
            "common.robot",
        ),
    );
    let r = extracted_call("Open Browser");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("common", Some("common.robot"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: false,
                alias_decode: None,
            },
            NameNormalization::None,
            &accept_any,
        )
        .expect("a non-wildcard import is scanned when wildcard_only is false");
    assert_eq!(resolved.target_symbol_id, 74);
}

#[test]
fn file_scoped_import_matches_under_normalization() {
    // A case-insensitive, space/underscore-stripping spec binds a Robot
    // keyword written in a different surface form ("openbrowser") to the
    // declared "Open Browser".
    let spec = NormSpec {
        case_insensitive: true,
        strip_chars: &[' ', '_'],
        strip_prefixes: &[],
        strip_sigils: &[],
    };
    let lookup = Lookup::new().with_in_file(
        "common.robot",
        sym(
            75,
            "Open Browser",
            "Open Browser",
            "function",
            "common.robot",
        ),
    );
    let r = extracted_call("openbrowser");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![wildcard_named_import("common", "common.robot")], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: None,
            },
            NameNormalization::Spec(spec),
            &accept_any,
        )
        .expect("normalized name binds across surface forms");
    assert_eq!(resolved.target_symbol_id, 75);
}

// ---------------------------------------------------------------------------
// FileScopedImports alias-decode — an import entry's `imported_name` is the
// lookup key and its `alias` names the bind target in the imported file.
// ---------------------------------------------------------------------------

const ROBOT_KW_SPEC: NormSpec = NormSpec {
    case_insensitive: true,
    strip_chars: &[' ', '_'],
    strip_prefixes: &[],
    strip_sigils: &[],
};

#[test]
fn alias_decode_binds_named_member() {
    // `alias = "Lib::add_to_cart"` decodes to the method `add_to_cart`. The
    // keyword's surface name (`Buy ${item}`) does NOT normalize to the method
    // name, so the plain symbol-name pass misses and the alias pass binds the
    // method over the owning class, at member confidence.
    let lookup = Lookup::new()
        .with_in_file("lib/cart.py", sym(80, "Lib", "Lib", "class", "lib/cart.py"))
        .with_in_file(
            "lib/cart.py",
            sym(81, "add_to_cart", "add_to_cart", "function", "lib/cart.py"),
        );
    let r = extracted_call("Buy ${item}");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![wildcard_alias_import(
            "buy${item}",
            Some("Lib::add_to_cart"),
            "lib/cart.py",
        )],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: Some(ALIAS_DECODE),
            },
            NameNormalization::Spec(ROBOT_KW_SPEC),
            &accept_any,
        )
        .expect("alias member binds");
    assert_eq!(resolved.target_symbol_id, 81);
    assert_eq!(resolved.strategy, "default_alias_decoded_import");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn alias_decode_binds_named_type() {
    // `alias = "AsyncLib"` (no member separator) decodes to the owning class.
    let lookup = Lookup::new().with_in_file(
        "lib/async.py",
        sym(82, "AsyncLib", "AsyncLib", "class", "lib/async.py"),
    );
    let r = extracted_call("Async Keyword");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![wildcard_alias_import(
            "asynckeyword",
            Some("AsyncLib"),
            "lib/async.py",
        )],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: Some(ALIAS_DECODE),
            },
            NameNormalization::Spec(ROBOT_KW_SPEC),
            &accept_any,
        )
        .expect("alias type binds");
    assert_eq!(resolved.target_symbol_id, 82);
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn alias_decode_no_alias_falls_back_to_fallback_kind() {
    // A keyword entry with no `alias` (module-level KEYWORDS dict) binds the
    // file's first `fallback_kind` symbol — the dispatch class.
    let lookup = Lookup::new().with_in_file(
        "lib/dyn.py",
        sym(83, "Dispatcher", "Dispatcher", "class", "lib/dyn.py"),
    );
    let r = extracted_call("One Arg");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![wildcard_alias_import("onearg", None, "lib/dyn.py")],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: Some(ALIAS_DECODE),
            },
            NameNormalization::Spec(ROBOT_KW_SPEC),
            &accept_any,
        )
        .expect("fallback binds the dispatch class");
    assert_eq!(resolved.target_symbol_id, 83);
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn alias_decode_off_does_not_match_imported_name() {
    // With `alias_decode: None`, a target equal to an import entry's
    // `imported_name` (not to any symbol NAME in the file) does NOT bind —
    // the plain symbol-name pass is the only behavior.
    let lookup = Lookup::new().with_in_file(
        "lib/async.py",
        sym(84, "AsyncLib", "AsyncLib", "class", "lib/async.py"),
    );
    let r = extracted_call("Async Keyword");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![wildcard_alias_import(
            "asynckeyword",
            Some("AsyncLib"),
            "lib/async.py",
        )],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: None
            },
            NameNormalization::Spec(ROBOT_KW_SPEC),
            &accept_any,
        )
        .is_none(),
        "without alias_decode an imported_name match must not bind"
    );
}

#[test]
fn alias_decode_member_missing_falls_through_to_type_then_fallback() {
    // The decoded member name isn't a symbol in the file, but the type is —
    // bind the type rather than failing.
    let lookup =
        Lookup::new().with_in_file("lib/cart.py", sym(85, "Lib", "Lib", "class", "lib/cart.py"));
    let r = extracted_call("Add To Cart");
    let s = source_symbol("caller");
    let fc = file_ctx(
        vec![wildcard_alias_import(
            "addtocart",
            Some("Lib::missing_method"),
            "lib/cart.py",
        )],
        None,
    );
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_file_scoped_import(
            FileScopedImports::On {
                wildcard_only: true,
                alias_decode: Some(ALIAS_DECODE),
            },
            NameNormalization::Spec(ROBOT_KW_SPEC),
            &accept_any,
        )
        .expect("falls through to the named type");
    assert_eq!(resolved.target_symbol_id, 85);
    assert_eq!(resolved.confidence, 1.0);
}

// ---------------------------------------------------------------------------
// alias_module_qname — a bare target equal to an import's bound name binds to
// the symbol whose qname IS the import's full module path (the module symbol,
// not a member). `alias MyApp.Foo` then a bare `Foo` → MyApp.Foo.
// ---------------------------------------------------------------------------

fn reject_module(_: EdgeKind, sym_kind: &str) -> bool {
    sym_kind != "module"
}

#[test]
fn alias_module_qname_off_is_inert() {
    let lookup = Lookup::new().with(sym(70, "Foo", "MyApp.Foo", "module", "lib/foo.ex"));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Foo", Some("MyApp.Foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_alias_module_qname(false, &accept_any)
        .is_none());
}

#[test]
fn alias_module_qname_binds_import_full_path_to_module_symbol() {
    let lookup = Lookup::new().with(sym(70, "Foo", "MyApp.Foo", "module", "lib/foo.ex"));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Foo", Some("MyApp.Foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_alias_module_qname(true, &accept_any)
        .expect("bare alias binds to the module symbol whose qname is the import path");
    assert_eq!(resolved.target_symbol_id, 70);
    assert_eq!(resolved.strategy, "default_alias_module_qname");
    assert_eq!(resolved.confidence, 1.0);
}

#[test]
fn alias_module_qname_declines_when_no_import_matches_target() {
    let lookup = Lookup::new().with(sym(70, "Foo", "MyApp.Foo", "module", "lib/foo.ex"));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Bar", Some("MyApp.Bar"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_alias_module_qname(true, &accept_any)
        .is_none());
}

#[test]
fn alias_module_qname_declines_kind_incompatible() {
    // The kind predicate rejects "module", so the alias bind must decline even
    // though the import's full path names a symbol of that kind.
    let lookup = Lookup::new().with(sym(70, "Foo", "MyApp.Foo", "module", "lib/foo.ex"));
    let r = extracted_call("Foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Foo", Some("MyApp.Foo"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(d
        .resolve_via_alias_module_qname(true, &reject_module)
        .is_none());
}

// ---------------------------------------------------------------------------
// namespaceless_global_type_lookup — flat-global first-match by-name bind
// (SQL and other namespaceless DDL/config languages)
// ---------------------------------------------------------------------------

/// Profile mirroring DEFAULT but opting into the namespaceless-global rung.
static NAMESPACELESS_PROFILE: LanguageProfile = LanguageProfile {
    namespaceless_global_type_lookup: NamespaceScope::Global,
    explicit_member_import: false,
    ..DEFAULT_PROFILE
};

/// Profile opting into the directory-scoped variant (Prisma).
static NAMESPACELESS_DIR_PROFILE: LanguageProfile = LanguageProfile {
    namespaceless_global_type_lookup: NamespaceScope::DirectoryScoped,
    explicit_member_import: false,
    ..DEFAULT_PROFILE
};

/// A TypeRef ref (the only edge SQL emits), bare-name, no scope/imports.
fn extracted_typeref(target: &str) -> ExtractedRef {
    let mut r = extracted_call(target);
    r.kind = EdgeKind::TypeRef;
    r
}

#[test]
fn namespaceless_global_binds_first_match_by_name() {
    // Two internal `struct` symbols both named `users`, in different files,
    // with distinct qnames — two genuinely separate candidates. The flat-global
    // rung binds the FIRST (id 1), where resolve_via_unique_internal_name
    // declines on the ambiguity.
    let lookup = Lookup::new()
        .with(sym(1, "users", "schema_a.users", "struct", "db/a.sql"))
        .with(sym(2, "users", "schema_b.users", "struct", "db/b.sql"));
    let r = extracted_typeref("users");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    // The non-unique sibling: first-match where the unique rung refuses.
    assert!(
        d.resolve_via_unique_internal_name(&accept_any).is_none(),
        "two candidates — the unique rung must decline"
    );
    let resolved = d
        .resolve_via_namespaceless_global(NamespaceScope::Global, &accept_any, &[])
        .expect("first-match binds among duplicate names");
    assert_eq!(resolved.target_symbol_id, 1, "binds the first candidate");
    assert_eq!(resolved.confidence, 1.0);
    assert_eq!(resolved.strategy, "default_namespaceless_global");
}

#[test]
fn namespaceless_global_skips_external_only_name() {
    // A name owned ONLY by an external file binds nothing — the rung is
    // internal-files-only.
    let mut ext = sym(5, "audit_log", "audit_log", "struct", "ext:db/vendor.sql");
    ext.file_path = Arc::from("ext:db/vendor.sql");
    let lookup = Lookup::new().with(ext);
    let r = extracted_typeref("audit_log");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_namespaceless_global(NamespaceScope::Global, &accept_any, &[])
            .is_none(),
        "external-only candidate must not bind"
    );
}

#[test]
fn namespaceless_global_strips_self_keyword_sigil() {
    // Flat-namespace sigil languages (Terraform `var.X`/`local.X`) keep the
    // sigil in the ref but declare the symbol bare; the rung retries the
    // self-keyword-stripped leaf.
    let lookup = Lookup::new().with(sym(7, "defaults", "defaults", "variable", "variables.tf"));
    let r = extracted_typeref("var.defaults");
    let s = source_symbol("main");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_via_namespaceless_global(NamespaceScope::Global, &accept_any, &["var", "local"])
        .expect("`var.defaults` binds to bare `defaults` after sigil strip");
    assert_eq!(resolved.target_symbol_id, 7);
    // Without the self keyword, the sigil'd target must NOT bind: `var.defaults`
    // has a `.`, but the leaf retry yields `defaults` — which DOES bind. So a
    // sigil-less probe binds via the generic leaf fallback, not the sigil strip.
    let leaf_bound = d
        .resolve_via_namespaceless_global(NamespaceScope::Global, &accept_any, &[])
        .expect("`var.defaults` leaf `defaults` binds via the dotted-leaf retry");
    assert_eq!(leaf_bound.target_symbol_id, 7);
}

#[test]
fn namespaceless_global_gate_default_inert() {
    // A default-profile language never reaches the rung in the full ladder for
    // the same duplicate-name fixture — the bool gate is default-off.
    let lookup = Lookup::new()
        .with(sym(1, "users", "schema_a.users", "struct", "db/a.sql"))
        .with(sym(2, "users", "schema_b.users", "struct", "db/b.sql"));
    let r = extracted_typeref("users");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_all_with_profile(&DEFAULT_PROFILE).is_none(),
        "gate default-off — the ladder must not first-match-bind"
    );
    // With the gate ON, the same fixture binds the first candidate through the
    // ladder.
    let bound = d
        .resolve_all_with_profile(&NAMESPACELESS_PROFILE)
        .expect("the gated rung binds through the full ladder");
    assert_eq!(bound.target_symbol_id, 1);
    assert_eq!(bound.strategy, "default_namespaceless_global");
}

// ---------------------------------------------------------------------------
// DirectoryScoped variant — a candidate binds only when it lives in the same
// directory as the referencing file (Prisma split-schema layout).
// ---------------------------------------------------------------------------

#[test]
fn dir_scoped_cross_directory_ref_does_not_bind() {
    // A Prisma model referenced from a sibling directory must NOT bind under
    // DirectoryScoped — guards the cross-schema-file regression. The only
    // candidate lives in `db/orders/`, the ref sits in `db/users/`.
    let lookup =
        Lookup::new().with(sym(1, "Account", "Account", "class", "db/orders/account.prisma"));
    let r = extracted_typeref("Account");
    let s = source_symbol("UsersModel");
    let fc = file_ctx_at("db/users/schema.prisma");
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_namespaceless_global(NamespaceScope::DirectoryScoped, &accept_any, &[])
            .is_none(),
        "cross-directory model ref must not first-match-bind under DirectoryScoped"
    );
    // The Global variant DOES bind it — proving the directory filter is what
    // declines, not a missing candidate.
    let global = d
        .resolve_via_namespaceless_global(NamespaceScope::Global, &accept_any, &[])
        .expect("Global binds the cross-directory candidate");
    assert_eq!(global.target_symbol_id, 1);
}

#[test]
fn dir_scoped_same_directory_ref_binds() {
    // A model in the SAME directory as the referencing file binds under
    // DirectoryScoped.
    let lookup =
        Lookup::new().with(sym(2, "Profile", "Profile", "class", "db/users/profile.prisma"));
    let r = extracted_typeref("Profile");
    let s = source_symbol("UsersModel");
    let fc = file_ctx_at("db/users/schema.prisma");
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let bound = d
        .resolve_via_namespaceless_global(NamespaceScope::DirectoryScoped, &accept_any, &[])
        .expect("same-directory model ref binds under DirectoryScoped");
    assert_eq!(bound.target_symbol_id, 2);
    assert_eq!(bound.strategy, "default_namespaceless_global");
}

#[test]
fn dir_scoped_binds_through_full_ladder() {
    // Same fixture, but exercised through the full ladder with the
    // DirectoryScoped profile — same-dir binds, cross-dir declines.
    let same_dir =
        Lookup::new().with(sym(2, "Profile", "Profile", "class", "db/users/profile.prisma"));
    let cross_dir =
        Lookup::new().with(sym(1, "Account", "Account", "class", "db/orders/account.prisma"));
    let s = source_symbol("UsersModel");
    let fc = file_ctx_at("db/users/schema.prisma");

    let r_same = extracted_typeref("Profile");
    let rc_same = ref_ctx(&r_same, &s, vec![]);
    let d_same = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc_same,
        lookup: &same_dir,
        kind_compatible: accept_any,
    };
    let bound = d_same
        .resolve_all_with_profile(&NAMESPACELESS_DIR_PROFILE)
        .expect("same-dir ref binds through the full ladder");
    assert_eq!(bound.target_symbol_id, 2);

    let r_cross = extracted_typeref("Account");
    let rc_cross = ref_ctx(&r_cross, &s, vec![]);
    let d_cross = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc_cross,
        lookup: &cross_dir,
        kind_compatible: accept_any,
    };
    assert!(
        d_cross
            .resolve_all_with_profile(&NAMESPACELESS_DIR_PROFILE)
            .is_none(),
        "cross-dir ref declines through the full ladder under DirectoryScoped"
    );
}

// ---------------------------------------------------------------------------
// Dotted-target leaf retry — a dotted miss retries its last `.`-segment, so a
// `schema.table` ref binds to a bare `table` symbol. Generic, not SQL-special.
// ---------------------------------------------------------------------------

#[test]
fn namespaceless_global_dotted_target_retries_leaf() {
    // A SQL `REFERENCES public.users` ref (dotted) resolves to a bare
    // `CREATE TABLE users` symbol via the generic last-segment retry — the
    // full `public.users` qname matches nothing.
    let lookup = Lookup::new().with(sym(3, "users", "users", "struct", "db/schema.sql"));
    let r = extracted_typeref("public.users");
    let s = source_symbol("orders");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let bound = d
        .resolve_via_namespaceless_global(NamespaceScope::Global, &accept_any, &[])
        .expect("dotted `public.users` retries leaf `users` and binds");
    assert_eq!(bound.target_symbol_id, 3);
    assert_eq!(bound.strategy, "default_namespaceless_global");
}

// ---------------------------------------------------------------------------
// Ladder-order invariant — a true scope/import hit must beat the coarse
// fallbacks (ambient package, unique-internal-name) that sit below it. These
// lock the ordering structurally: they fail only if a future edit reshuffles a
// coarse rung above scope/import, or wires the unwired unique-name rung back
// into the ladder.
// ---------------------------------------------------------------------------

/// The same bare name `Widget` is reachable BOTH via an imported namespace
/// (`import Widgets from "pkg"` → qname `pkg.Widget`) AND via a declared
/// ambient package (`ambient.Widget` on an `is_ambient_path` file). The
/// import-scoped rung (`resolve_via_imported_namespace`, ladder position 15)
/// runs ABOVE the ambient-package fallback (position 16), so the full ladder
/// must bind the import candidate — never the coarse ambient one.
#[test]
fn scope_import_hit_beats_coarse_ambient_fallback() {
    let lookup = Lookup::new()
        .with(sym(
            300,
            "Widget",
            "pkg.Widget",
            "function",
            "src/widgets.ts",
        ))
        .with(sym(
            301,
            "Widget",
            "ambient.Widget",
            "function",
            "ext:node_modules/ambient/index.d.ts",
        ))
        .with_ambient("ext:node_modules/ambient/index.d.ts");
    let r = extracted_call("Widget");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![import("Widgets", Some("pkg"))], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    // The ambient fallback is genuinely LIVE for this fixture: probed alone it
    // binds the ambient candidate. Without this the order test could pass
    // vacuously (the fallback being inert rather than the import winning a race).
    let ambient_alone = d
        .resolve_via_ambient_package(&accept_any)
        .expect("ambient fallback is live for this fixture");
    assert_eq!(ambient_alone.target_symbol_id, 301);
    assert_eq!(ambient_alone.strategy, "default_ambient_package");
    // Through the full ladder the import rung above it wins.
    let resolved = d
        .resolve_all_with_profile(&crate::languages::java::JAVA_PROFILE)
        .expect("the import-scoped candidate binds through the ladder");
    assert_eq!(
        resolved.target_symbol_id, 300,
        "import candidate must win, not the ambient fallback"
    );
    assert_eq!(
        resolved.strategy, "default_imported_namespace",
        "the import rung above ambient_package must be the binder"
    );
}

/// `resolve_via_unique_internal_name` is defined but deliberately UNWIRED from
/// the ladder. With two same-named internal candidates the method declines on
/// the ambiguity; with a single candidate it WOULD bind — but neither path is
/// reachable through `resolve_all_with_profile`, so the full ladder never
/// returns a `default_unique_internal_name` binding. Locks the rung out.
#[test]
fn unique_internal_name_stays_out_of_ladder() {
    // Two same-named internal candidates: the by-name rung declines on ambiguity.
    let ambiguous = Lookup::new()
        .with(sym(310, "doIt", "Foo.doIt", "function", "src/a.rs"))
        .with(sym(311, "doIt", "Bar.doIt", "function", "src/b.rs"));
    let r = extracted_call("doIt");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &ambiguous,
        kind_compatible: accept_any,
    };
    assert!(
        d.resolve_via_unique_internal_name(&accept_any).is_none(),
        "ambiguity must not be guessed by the unwired rung"
    );
    assert!(
        d.resolve_all_with_profile(&DEFAULT_PROFILE).is_none()
            || d.resolve_all_with_profile(&DEFAULT_PROFILE)
                .map(|res| res.strategy != "default_unique_internal_name")
                .unwrap_or(true),
        "the unique-internal-name rung is not part of the ladder"
    );

    // A SINGLE internal candidate: the method WOULD bind directly, proving the
    // rung is functional — yet the ladder still must not route through it.
    let unique = Lookup::new().with(sym(
        312,
        "onlyOne",
        "Solo.onlyOne",
        "function",
        "src/solo.rs",
    ));
    let r2 = extracted_call("onlyOne");
    let rc2 = ref_ctx(&r2, &s, vec![]);
    let d2 = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc2,
        lookup: &unique,
        kind_compatible: accept_any,
    };
    assert_eq!(
        d2.resolve_via_unique_internal_name(&accept_any)
            .expect("single candidate binds when called directly")
            .target_symbol_id,
        312,
        "the rung itself is functional",
    );
    assert!(
        d2.resolve_all_with_profile(&DEFAULT_PROFILE)
            .map(|res| res.strategy != "default_unique_internal_name")
            .unwrap_or(true),
        "even a single internal candidate must not bind via the unwired rung",
    );
}

// ---------------------------------------------------------------------------
// resolve_via_implicit_prelude — language implicit-import binding
// ---------------------------------------------------------------------------

fn run_implicit_prelude(
    lookup: &Lookup,
    target: &str,
    namespaces: &[&str],
    separator: &str,
) -> Option<Resolution> {
    let r = extracted_call(target);
    let s = source_symbol("Host");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind_compatible: accept_any,
    };
    d.resolve_via_implicit_prelude(namespaces, separator, &accept_any)
}

#[test]
fn implicit_prelude_binds_direct_member() {
    // Bare `String` binds to the direct member `java.lang.String`.
    let lookup = Lookup::new().with(sym(
        7,
        "String",
        "java.lang.String",
        "class",
        "ext:/java.lang/String.java",
    ));
    let res =
        run_implicit_prelude(&lookup, "String", &["java.lang"], ".").expect("direct member binds");
    assert_eq!(res.target_symbol_id, 7);
    assert_eq!(res.strategy, "implicit_prelude");
}

#[test]
fn implicit_prelude_rejects_nested_method_and_subnamespace() {
    // Nested type, method, and sub-namespace all carry a further `.` segment
    // after `java.lang.` and must NOT bind to a bare name.
    let lookup = Lookup::new()
        .with(sym(
            1,
            "Controller",
            "java.lang.ModuleLayer.Controller",
            "class",
            "x",
        ))
        .with(sym(2, "get", "java.lang.ClassValue.get", "method", "x"))
        .with(sym(
            3,
            "Configuration",
            "java.lang.module.Configuration",
            "class",
            "x",
        ));
    assert!(run_implicit_prelude(&lookup, "Controller", &["java.lang"], ".").is_none());
    assert!(run_implicit_prelude(&lookup, "get", &["java.lang"], ".").is_none());
    assert!(run_implicit_prelude(&lookup, "Configuration", &["java.lang"], ".").is_none());
}

#[test]
fn implicit_prelude_honours_non_dot_separator() {
    // R qnames use `::`. `base::c` binds under namespace `base`; the sibling
    // `boot::c` is excluded, so the single hit is unambiguous.
    let lookup = Lookup::new()
        .with(sym(10, "c", "base::c", "function", "ext:/r/base.R"))
        .with_overload(sym(11, "c", "boot::c", "function", "ext:/r/boot.R"));
    let res = run_implicit_prelude(&lookup, "c", &["base"], "::").expect("base::c binds");
    assert_eq!(res.target_symbol_id, 10);
}

#[test]
fn implicit_prelude_declines_without_namespaces() {
    let lookup = Lookup::new().with(sym(7, "String", "java.lang.String", "class", "x"));
    assert!(run_implicit_prelude(&lookup, "String", &[], ".").is_none());
}

#[test]
fn implicit_prelude_dedups_duplicate_qname_rows() {
    // A hydrated stdlib often ships several rows for one symbol (declaration
    // merging / multiple jars). Same qname → same symbol, not ambiguity.
    let lookup = Lookup::new()
        .with(sym(
            20,
            "Map",
            "kotlin.collections.Map",
            "interface",
            "ext:/k/Map.kt",
        ))
        .with_overload(sym(
            21,
            "Map",
            "kotlin.collections.Map",
            "interface",
            "ext:/k/Map2.kt",
        ));
    let res = run_implicit_prelude(&lookup, "Map", &["kotlin.collections"], ".")
        .expect("duplicate-qname rows are one symbol, must bind");
    assert_eq!(res.target_symbol_id, 20);
}

#[test]
fn implicit_prelude_declines_two_distinct_member_qnames() {
    // Two DISTINCT direct members of declared namespaces share the bare name —
    // a genuine ambiguity, decline.
    let lookup = Lookup::new()
        .with(sym(30, "Map", "kotlin.collections.Map", "interface", "x"))
        .with_overload(sym(31, "Map", "kotlin.io.Map", "class", "x"));
    assert!(
        run_implicit_prelude(&lookup, "Map", &["kotlin.collections", "kotlin.io"], ".").is_none(),
        "two distinct implicit-member qnames must decline"
    );
}

#[test]
fn nix_profile_resolves_let_aliased_head_to_in_file_binding() {
    // `let l = lib; in l.mkOption` — the dotted target's head `l` names the
    // same-file `let` binding that aliases `lib`. The head-alias rung truncates
    // at the first `.` and binds the head to the in-file `l` declaration, so
    // `l.mkOption` reads against `lib`.
    let lookup = Lookup::new().with_in_file(
        "default.nix",
        sym(90, "l", "l", "variable", "default.nix"),
    );
    let r = extracted_call("l.mkOption");
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = FileContext {
        file_path: "default.nix".to_string(),
        language: "nix".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d
        .resolve_all_with_profile(&crate::languages::nix::NIX_PROFILE)
        .expect("nix let-alias head binds through the ladder");
    assert_eq!(resolved.target_symbol_id, 90);
    assert_eq!(resolved.strategy, "default_head_alias");
}
