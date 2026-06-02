// =============================================================================
// type_checker/core/default_resolver_tests.rs — unit tests for DefaultResolver.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolInfo, SymbolLookup};
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind,
    SymbolKind, Visibility,
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
        }
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
        self.generics
            .insert(qname.to_string(), params.iter().map(|s| s.to_string()).collect());
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
    fn with_reexport(
        mut self,
        target: &str,
        prefix: &str,
        module: &str,
        sym_id: i64,
    ) -> Self {
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
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
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
    ExtractedRef { is_import_binding: false, is_reexport: false,
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
    let resolved = d.resolve_via_ref_module().expect("module-qualified resolves");
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
    let resolved = d.resolve_via_ref_module().expect("file-stem match resolves");
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
    assert!(d.resolve_via_ref_module().is_none());
}

#[test]
fn qname_exact_resolves_dotted_target() {
    let lookup =
        Lookup::new().with(sym(3, "List", "Catalog.Services.List", "function", "src/svc.cs"));
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
    let resolved = d.resolve_via_qname_exact().expect("dotted target resolves");
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
    assert!(d.resolve_via_qname_exact().is_none());
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
    let resolved = d.resolve_via_file_import().expect("import resolves");
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
    let resolved = d.resolve_via_file_import().expect("alias maps to original");
    assert_eq!(resolved.target_symbol_id, 12);
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
        vec![import("eShop.Catalog.API.Model", Some("eShop.Catalog.API.Model"))],
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
        .resolve_via_namespace_import()
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
        .resolve_via_ambient_namespace_path()
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
    let resolved = d.resolve_via_same_namespace().expect("same namespace resolves");
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
    assert!(d.resolve_via_same_namespace().is_none());
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
        .resolve_via_imported_namespace()
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
    let resolved = d.resolve_via_chain_prefix().expect("chain prefix resolves");
    assert_eq!(resolved.target_symbol_id, 50);
    assert_eq!(resolved.strategy, "default_chain_prefix");
}

#[test]
fn scope_visible_resolves_against_innermost_scope_first() {
    let lookup = Lookup::new()
        .with(sym(60, "helper", "outer.helper", "function", "src/outer.rs"))
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
    let rc = ref_ctx(
        &r,
        &s,
        vec!["outer.inner".to_string(), "outer".to_string()],
    );
    let d = DefaultResolver {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind_compatible: accept_any,
    };
    let resolved = d.resolve_via_scope_visible().expect("scope walk resolves");
    assert_eq!(resolved.target_symbol_id, 61, "innermost scope wins");
    assert_eq!(resolved.strategy, "default_scope_visible");
}

#[test]
fn kind_compatible_filter_rejects_class_for_calls_when_strict() {
    fn strict_calls(edge: EdgeKind, sym_kind: &str) -> bool {
        match edge {
            EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor"),
            _ => true,
        }
    }
    let lookup =
        Lookup::new().with(sym(70, "Foo", "Foo", "class", "src/foo.ts"));
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
        d.resolve_via_file_import().is_none(),
        "strict kind filter rejects class for calls"
    );
}

#[test]
fn resolve_all_prefers_innermost_scope_over_imports() {
    // Two viable targets: one in the innermost scope, one via file import.
    // Canonical order says scope wins.
    let lookup = Lookup::new()
        .with(sym(80, "helper", "outer.helper", "function", "src/outer.rs"))
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
        .resolve_via_unique_internal_name()
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
        d.resolve_via_unique_internal_name().is_none(),
        "ambiguity must not be guessed"
    );
}

#[test]
fn unique_internal_name_ignores_external_candidates() {
    let mut ext_sym = sym(203, "load", "vendor.load", "function", "ext:vendor/lib.d.ts");
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
        .resolve_via_unique_internal_name()
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
    let resolved = d.resolve_via_same_file().expect("same-file sibling resolves");
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
    let lookup = Lookup::new()
        .with(sym.clone())
        .with_reexport(
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
        .resolve_via_reexport_chain()
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
    let lookup = Lookup::new()
        .with(sym_inner)
        .with_reexport(
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
        .resolve_via_reexport_chain()
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
    assert!(d.resolve_via_reexport_chain().is_none());
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
        .resolve_via_ambient_package()
        .expect("ambient package preferred");
    assert_eq!(resolved.target_symbol_id, 100);
    assert_eq!(resolved.strategy, "default_ambient_package");
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
    assert!(d.resolve_via_ambient_package().is_none());
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
        .resolve_via_self_keyword()
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
        .resolve_via_self_keyword()
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
    assert!(d.resolve_via_self_keyword().is_none());
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
        .resolve_via_enclosing_member()
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
    assert!(d.resolve_via_enclosing_member().is_none());
}

// ---------------------------------------------------------------------------
// resolve_via_aliased_import — specifier rewritten through a path alias
// ---------------------------------------------------------------------------

#[test]
fn aliased_import_resolves_via_path_alias() {
    let lookup = Lookup::new()
        .with(sym(330, "helper", "helper", "function", "src/utils/helper.ts"))
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
        .resolve_via_aliased_import()
        .expect("aliased specifier resolves after rewrite");
    assert_eq!(resolved.target_symbol_id, 330);
    assert_eq!(resolved.strategy, "engine_aliased_import");
}

#[test]
fn aliased_import_none_without_rewrite() {
    // No alias registered → resolve_path_alias returns None → stay out,
    // leaving the raw-path case to resolve_via_file_import.
    let lookup =
        Lookup::new().with(sym(331, "helper", "helper", "function", "src/utils/helper.ts"));
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
    assert!(d.resolve_via_aliased_import().is_none());
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
        Self { inner: lookup, workspace_packages: Default::default() }
    }
    fn with_workspace_package(mut self, specifier: &str, id: i64) -> Self {
        self.workspace_packages.insert(specifier.to_string(), id);
        self
    }
}

impl SymbolLookup for WorkspaceLookup {
    fn by_name(&self, n: &str) -> &[SymbolInfo] { self.inner.by_name(n) }
    fn by_qualified_name(&self, q: &str) -> Option<&SymbolInfo> { self.inner.by_qualified_name(q) }
    fn members_of(&self, p: &str) -> &[SymbolInfo] { self.inner.members_of(p) }
    fn types_by_name(&self, n: &str) -> &[SymbolInfo] { self.inner.types_by_name(n) }
    fn in_namespace(&self, n: &str) -> Vec<&SymbolInfo> { self.inner.in_namespace(n) }
    fn has_in_namespace(&self, n: &str) -> bool { self.inner.has_in_namespace(n) }
    fn in_file(&self, p: &str) -> &[SymbolInfo] { self.inner.in_file(p) }
    fn field_type_name(&self, q: &str) -> Option<&str> { self.inner.field_type_name(q) }
    fn return_type_name(&self, q: &str) -> Option<&str> { self.inner.return_type_name(q) }
    fn field_type_args(&self, q: &str) -> Option<&[String]> { self.inner.field_type_args(q) }
    fn generic_params(&self, q: &str) -> Option<&[String]> { self.inner.generic_params(q) }
    fn reexports_from(&self, p: &str) -> &[(String, String)] { self.inner.reexports_from(p) }
    fn is_external_name(&self, n: &str, l: &str) -> bool { self.inner.is_external_name(n, l) }
    fn is_ambient_path(&self, p: &str) -> bool { self.inner.is_ambient_path(p) }
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
    assert!(d.resolve_via_ranked_candidates().is_none());
}

#[test]
fn ranked_picks_same_workspace_package_over_external() {
    // Two `Foo` candidates: one internal (workspace pkg 42), one external.
    // The same-package bonus (+1000) dominates everything else.
    let lookup = Lookup::new()
        .with(sym_full(11, "Foo", "internal.Foo", "class", "src/internal/foo.ts", Some("public"), Some(42)))
        .with(sym_full(22, "Foo", "ext.Foo", "class", "ext:idx:/cache/somelib/foo.d.ts", Some("public"), None));
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
    let resolved = d.resolve_via_ranked_candidates().expect("ranked picks workspace match");
    assert_eq!(resolved.target_symbol_id, 11);
    assert_eq!(resolved.strategy, "default_ranked_candidate");
}

#[test]
fn ranked_picks_imported_package_over_random_externals() {
    // Three external `expect` candidates from different packages. Caller
    // imports from `@types/jest` — workspace_package_id resolves that to id=7.
    // Only the jest-attributed candidate has package_id=7, so it wins.
    let inner = Lookup::new()
        .with(sym_full(1, "expect", "expect", "function", "ext:/cache/@types/jest/index.d.ts", Some("public"), Some(7)))
        .with(sym_full(2, "expect", "expect", "function", "ext:/cache/@types/vitest/dist/index.d.ts", Some("public"), Some(8)))
        .with(sym_full(3, "expect", "expect", "function", "ext:/cache/@types/chai/index.d.ts", Some("public"), Some(9)));
    let lookup = WorkspaceLookup::new(inner)
        .with_workspace_package("@types/jest", 7);

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
    let resolved = d.resolve_via_ranked_candidates().expect("ranked picks imported package");
    assert_eq!(resolved.target_symbol_id, 1);
}

#[test]
fn ranked_returns_none_when_top_two_tie() {
    // Two indistinguishable external candidates — both public, same depth,
    // neither imported, no package match. Margin not met → stay None.
    let lookup = Lookup::new()
        .with(sym_full(101, "Foo", "Foo", "class", "ext:/cache/a/foo.d.ts", Some("public"), None))
        .with(sym_full(102, "Foo", "Foo", "class", "ext:/cache/b/foo.d.ts", Some("public"), None));
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
    assert!(d.resolve_via_ranked_candidates().is_none());
}

#[test]
fn ranked_prefers_ambient_path() {
    // One ambient candidate (TS @types-style), one non-ambient. Ambient
    // bonus (+200) plus public-visibility tilts the win to the ambient one.
    let lookup = Lookup::new()
        .with(sym_full(50, "expect", "expect", "function", "ext:/cache/@types/jest/index.d.ts", Some("public"), None))
        .with(sym_full(51, "expect", "expect", "function", "ext:/cache/some-other/expect.d.ts", Some("public"), None))
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
    let resolved = d.resolve_via_ranked_candidates().expect("ranked picks ambient");
    assert_eq!(resolved.target_symbol_id, 50);
}

#[test]
fn ranked_penalises_private_external_candidates() {
    // One private external, one public external. The public one wins despite
    // identical paths — public+50 minus private-200 = 250-point gap.
    let lookup = Lookup::new()
        .with(sym_full(70, "Foo", "Foo", "class", "ext:/cache/somepkg/foo.d.ts", Some("private"), None))
        .with(sym_full(71, "Foo", "Foo", "class", "ext:/cache/somepkg/bar.d.ts", Some("public"), None));
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
    let resolved = d.resolve_via_ranked_candidates().expect("ranked picks public");
    assert_eq!(resolved.target_symbol_id, 71);
}

#[test]
fn ranked_picks_via_qname_prefix_when_import_module_matches() {
    // Caller imports `lodash` — three lodash-named externals at different
    // qname paths. The one whose qname literally starts with `lodash.` wins
    // via the +300 prefix bonus.
    let lookup = Lookup::new()
        .with(sym_full(200, "map", "lodash.map", "function", "ext:/cache/@types/lodash/index.d.ts", Some("public"), None))
        .with(sym_full(201, "map", "rxjs.operators.map", "function", "ext:/cache/rxjs/operators.d.ts", Some("public"), None))
        .with(sym_full(202, "map", "other.helper.map", "function", "ext:/cache/other/helper.d.ts", Some("public"), None));
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
    let resolved = d.resolve_via_ranked_candidates().expect("ranked follows import");
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
        (extracted_call("Foo"), file_ctx(vec![import("Foo", Some("./foo"))], None)),
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
            .resolve_via_ref_module()
            .or_else(|| d.resolve_via_file_import())
            .expect("one of the strategies resolves");
        assert_eq!(resolved.confidence, 1.0, "deterministic, never decayed");
    }
}
