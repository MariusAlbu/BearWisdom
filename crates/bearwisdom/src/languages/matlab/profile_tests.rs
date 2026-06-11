// =============================================================================
// matlab/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// MATLAB rides the generic ladder with no resolve_ref hook. Two pure-data
// flags on MATLAB_PROFILE drive its bare-call binds:
//   * module_scope == SameDir — same-folder function sibling (MATLAB path
//     precedence) binds via the same-dir rung, which runs first.
//   * namespaceless_global_type_lookup — a cross-dir project function with no
//     same-dir sibling first-match-binds via the dead-last rung; toolbox
//     intrinsics (external) decline and stay external.
// =============================================================================

use super::MATLAB_PROFILE;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::types::*;
use std::collections::HashMap;

#[test]
fn matlab_profile_identity_and_shadow_mode() {
    assert_eq!(MATLAB_PROFILE.id, "matlab");
}

#[test]
fn matlab_module_scope_is_same_dir() {
    // MATLAB path semantics: a same-folder function sibling is the canonical
    // bind for a bare call. The same-dir rung is selected by SameDir.
    assert_eq!(
        MATLAB_PROFILE.module_scope,
        crate::type_checker::profile::language_profile::ModuleScope::SameDir
    );
}

#[test]
fn matlab_namespaceless_global_is_on() {
    // A bare call to a cross-dir project function with no same-dir sibling
    // falls through to the dead-last first-match-by-name rung.
    assert_eq!(
        MATLAB_PROFILE.namespaceless_global_type_lookup,
        crate::type_checker::profile::language_profile::NamespaceScope::Global
    );
}

// ---------------------------------------------------------------------------
// Full-ladder bind tests through resolve_all_with_profile(&MATLAB_PROFILE).
// ---------------------------------------------------------------------------

fn accept_any(_edge: EdgeKind, _sym_kind: &str) -> bool {
    true
}

fn make_sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_call(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 2,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "matlab".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 10,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn clone_pf(f: &ParsedFile) -> ParsedFile {
    make_file(&f.path, f.symbols.clone(), f.refs.clone())
}

fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files.iter().map(|f| clone_pf(f)).collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

fn sym_id(id_map: &HashMap<(String, String), i64>, file: &str, name: &str) -> i64 {
    *id_map
        .get(&(file.to_string(), name.to_string()))
        .unwrap_or_else(|| panic!("symbol not found: {file}::{name}"))
}

fn resolve_call(file_path: &str, target: &str, all: &[&ParsedFile]) -> Option<Resolution> {
    let (index, _) = build_index(all);
    let caller = make_file(
        file_path,
        vec![make_sym("caller", SymbolKind::Function)],
        vec![make_call(target)],
    );
    let file_ctx = FileContext {
        file_path: file_path.to_string(),
        language: "matlab".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let r = &caller.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &index,
        kind_compatible: accept_any,
    }
    .resolve_all_with_profile(&MATLAB_PROFILE)
}

#[test]
fn matlab_bare_call_binds_cross_dir_internal_via_namespaceless_global() {
    // Two internal `NDSort` functions in different algorithm folders plus an
    // ambient toolbox stub under ext:. With no same-dir sibling for the caller,
    // the bare call falls dead-last to the first-match-by-name rung and binds
    // an INTERNAL symbol, never the external stub.
    let a = make_file(
        "algorithms/NSGAII/NDSort.m",
        vec![make_sym("NDSort", SymbolKind::Function)],
        vec![],
    );
    let b = make_file(
        "algorithms/SPEA2/NDSort.m",
        vec![make_sym("NDSort", SymbolKind::Function)],
        vec![],
    );
    let ext = make_file(
        "ext:matlab:toolbox/NDSort.m",
        vec![make_sym("NDSort", SymbolKind::Function)],
        vec![],
    );
    let (id_a, id_b, ext_id) = {
        let (_, id_map) = build_index(&[&a, &b, &ext]);
        (
            sym_id(&id_map, "algorithms/NSGAII/NDSort.m", "NDSort"),
            sym_id(&id_map, "algorithms/SPEA2/NDSort.m", "NDSort"),
            sym_id(&id_map, "ext:matlab:toolbox/NDSort.m", "NDSort"),
        )
    };
    let res = resolve_call("algorithms/MOEAD/MOEAD.m", "NDSort", &[&a, &b, &ext])
        .expect("cross-dir bare call binds an internal function");
    assert_eq!(res.strategy, "default_namespaceless_global");
    assert_ne!(
        res.target_symbol_id, ext_id,
        "must not bind the ext toolbox stub"
    );
    assert!(
        res.target_symbol_id == id_a || res.target_symbol_id == id_b,
        "binds an internal NDSort (got {})",
        res.target_symbol_id
    );
}

#[test]
fn matlab_same_dir_sibling_wins_over_cross_dir() {
    // When a same-folder sibling exists, MATLAB path precedence binds it first:
    // the same-dir rung runs ahead of the namespaceless-global rung.
    let sibling = make_file(
        "algorithms/NSGAII/CalFitness.m",
        vec![make_sym("CalFitness", SymbolKind::Function)],
        vec![],
    );
    let other = make_file(
        "algorithms/SPEA2/CalFitness.m",
        vec![make_sym("CalFitness", SymbolKind::Function)],
        vec![],
    );
    let (id_sibling, _) = {
        let (_, id_map) = build_index(&[&sibling, &other]);
        (
            sym_id(&id_map, "algorithms/NSGAII/CalFitness.m", "CalFitness"),
            (),
        )
    };
    let res = resolve_call(
        "algorithms/NSGAII/main.m",
        "CalFitness",
        &[&sibling, &other],
    )
    .expect("same-dir sibling resolves");
    assert_eq!(res.strategy, "default_same_dir");
    assert_eq!(
        res.target_symbol_id, id_sibling,
        "binds the same-folder sibling"
    );
}
