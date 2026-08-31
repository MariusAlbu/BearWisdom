// =============================================================================
// engine/module_entry_tests.rs — declared-module keys in the Compilation
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry, SymbolLookup};
use crate::indexer::resolve::engine::root_import_discipline::{apply, RootImportOutcome};
use crate::indexer::resolve::engine::cause::CauseKind;
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    ChainSegment, ExtractedSymbol, FlowMeta, ParsedFile, SegmentKind, SymbolKind, Visibility,
};

fn make_symbol(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
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

fn make_parsed_file(
    path: &str,
    symbols: Vec<ExtractedSymbol>,
    declared_modules: Vec<String>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
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
        declared_modules,
    }
}

/// A Compilation over one internal shim file declaring `virtual:pwa` (and a
/// `*.css` wildcard pattern) with one class the module body exports.
fn build_shim_compilation() -> (Compilation, Arc<TypeArena>) {
    let arena = Arc::new(TypeArena::new());
    let pf = make_parsed_file(
        "src/shims.d.ts",
        vec![make_symbol("RegisterOptions", "RegisterOptions", SymbolKind::Class)],
        vec!["virtual:pwa".to_string(), "*.css".to_string()],
    );
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/shims.d.ts".to_string(), "RegisterOptions".to_string()), 11);
    let tree = Compilation::build_with_context(
        &[pf],
        &id_map,
        Arc::clone(&arena),
        None,
        &HashSet::new(),
    );
    (tree, arena)
}

fn seg(name: &str) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

fn ctx_with(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/app.ts".to_string(),
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
fn declared_modules_produce_module_entry_keys() {
    let (tree, _) = build_shim_compilation();
    assert_eq!(
        tree.resolve_module_from("src/app.ts", "virtual:pwa"),
        Some("src/shims.d.ts"),
        "a declared ambient-module name must key the declaring file"
    );
    assert_eq!(
        tree.resolve_module_from("src/app.ts", "*.css"),
        None,
        "a wildcard declaration pattern is not an exact specifier and gets no key"
    );
}

#[test]
fn import_ref_against_declared_specifier_resolves() {
    let (tree, arena) = build_shim_compilation();
    let ctx = ctx_with(vec![imp("RegisterOptions", "virtual:pwa")]);
    match apply(&ctx, &tree, &arena, &seg("RegisterOptions")) {
        RootImportOutcome::Typed(recv) => assert_eq!(
            recv.id,
            Some(11),
            "the root must type from the declaring file's own symbol"
        ),
        _ => panic!("an import bound to a declared module specifier must type the root"),
    }
}

#[test]
fn scheme_specifier_without_key_still_denies() {
    let (tree, arena) = build_shim_compilation();
    let ctx = ctx_with(vec![imp("thing", "virtual:unregistered")]);
    match apply(&ctx, &tree, &arena, &seg("thing")) {
        RootImportOutcome::Deny(c) => assert_eq!(c.kind, CauseKind::ImportUnlinked),
        _ => panic!("a scheme-prefixed specifier with no module key must keep denying"),
    }
}
