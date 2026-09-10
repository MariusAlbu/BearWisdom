// =============================================================================
// engine/module_entry_tests.rs — declared-module keys in the Compilation
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::indexer::resolve::engine::cause::CauseKind;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry, SymbolLookup};
use crate::indexer::resolve::engine::root_import_discipline::{apply, RootImportOutcome};
use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};
use crate::types::{
    ChainSegment, ExtractedSymbol, FlowMeta, ParsedFile, SegmentKind, SymbolKind, Visibility,
};

static MODULE_PROFILE: LanguageProfile = LanguageProfile {
    id: "typescript",
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        module_prefix_rewrites:
            crate::type_checker::profile::language_profile::ModulePrefixRewrites::On {
                module_path_adapter: Some(crate::ecosystem::npm::node_builtin::module_path_match),
                candidate_prefixes:
                    crate::ecosystem::npm::module_specifier::module_prefix_candidates,
                declines_directory_match:
                    crate::ecosystem::npm::module_specifier::declines_directory_match,
            },
        ..DEFAULT_PROFILE.imports
    },
    ..DEFAULT_PROFILE
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
    make_parsed_file_for_language(path, "typescript", symbols, declared_modules)
}

fn make_parsed_file_for_language(
    path: &str,
    language: &str,
    symbols: Vec<ExtractedSymbol>,
    declared_modules: Vec<String>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
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

/// A Compilation over one internal fixture declaring `virtual:pwa` (and a
/// wildcard pattern) with one class the module body exports.
fn build_shim_compilation() -> (Compilation, Arc<TypeArena>) {
    let arena = Arc::new(TypeArena::new());
    let pf = make_parsed_file(
        "src/shims.fixture",
        vec![make_symbol(
            "RegisterOptions",
            "RegisterOptions",
            SymbolKind::Class,
        )],
        vec!["virtual:pwa".to_string(), "*.css".to_string()],
    );
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        (
            "src/shims.fixture".to_string(),
            "RegisterOptions".to_string(),
        ),
        11,
    );
    let tree = Compilation::build_with_context(
        &[pf],
        &id_map.clone().into(),
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
        file_path: "src/app.fixture".to_string(),
        language: "fixture".to_string(),
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
        binding_kind: None,
    }
}

#[test]
fn declared_modules_produce_module_entry_keys() {
    let (tree, _) = build_shim_compilation();
    assert_eq!(
        tree.resolve_module_from("src/app.fixture", "virtual:pwa"),
        Some("src/shims.fixture"),
        "a declared ambient-module name must key the declaring file"
    );
    assert_eq!(
        tree.resolve_module_from("src/app.fixture", "*.css"),
        None,
        "a wildcard declaration pattern is not an exact specifier and gets no key"
    );
}

#[test]
fn non_owner_declared_module_metadata_cannot_create_an_entry() {
    let arena = Arc::new(TypeArena::new());
    let pf = make_parsed_file_for_language(
        "src/non_owner.rs",
        "rust",
        vec![],
        vec!["virtual:pwa".to_string()],
    );
    let tree = Compilation::build_with_context(
        &[pf],
        &HashMap::new().into(),
        arena,
        None,
        &HashSet::new(),
    );
    assert_eq!(tree.resolve_module_from("src/app.rs", "virtual:pwa"), None);
}

#[test]
fn import_ref_against_declared_specifier_resolves() {
    let (tree, arena) = build_shim_compilation();
    let ctx = ctx_with(vec![imp("RegisterOptions", "virtual:pwa")]);
    match apply(
        &ctx,
        &tree,
        &arena,
        &MODULE_PROFILE,
        &seg("RegisterOptions"),
    ) {
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
    match apply(&ctx, &tree, &arena, &MODULE_PROFILE, &seg("thing")) {
        RootImportOutcome::Deny(c) => assert_eq!(c.kind, CauseKind::ImportUnlinked),
        _ => panic!("a scheme-prefixed specifier with no module key must keep denying"),
    }
}
