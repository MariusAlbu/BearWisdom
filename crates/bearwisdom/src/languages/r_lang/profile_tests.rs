// =============================================================================
// r_lang/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// R is a flat top-level function namespace: a project's own exported functions
// (defined in sibling .R files) are called bare with no scope/import structure.
// `namespaceless_global_type_lookup` binds such a bare call first-match to the
// project-internal definition; a same-named external stub (testthat/base) loses
// because the rung excludes external files. Qualified `pkg::fn` calls carry the
// `::` separator and stay external (the bare-name rung never sees them).
// =============================================================================

use super::R_PROFILE;
use crate::indexer::resolve::legacy::{FileContext, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::language_profile::DispatchAxis;
use crate::types::*;
use std::collections::HashMap;

#[test]
fn r_profile_identity_and_shadow_mode() {
    assert_eq!(R_PROFILE.id, "r");
    assert_eq!(R_PROFILE.qname_separator, "::");
}

#[test]
fn r_dispatch_axis_is_multi_arg_for_s4() {
    assert_eq!(R_PROFILE.dispatch_axis, DispatchAxis::MultiArg);
}

#[test]
fn r_namespaceless_global_is_on() {
    // R's flat function namespace binds bare calls to the project's own
    // exported functions via the dead-last first-match-by-name rung.
    assert_eq!(
        R_PROFILE.namespaceless_global_type_lookup,
        crate::type_checker::profile::language_profile::NamespaceScope::Global
    );
}

// ---------------------------------------------------------------------------
// Full-ladder bind tests through resolve_all_with_profile(&R_PROFILE).
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
        language: "r".to_string(),
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
        language: "r".to_string(),
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
    .resolve_all_with_profile(&R_PROFILE)
}

#[test]
fn r_bare_call_binds_internal_function_over_external_stub() {
    // `group_by` is the project's own exported function (two defs across .R
    // files) plus a same-named ext:r-stdlib stub. The bare call binds an
    // INTERNAL definition first-match; the external stub never wins.
    let a = make_file(
        "R/group_by.R",
        vec![make_sym("group_by", SymbolKind::Function)],
        vec![],
    );
    let b = make_file(
        "R/grouped_df.R",
        vec![make_sym("group_by", SymbolKind::Function)],
        vec![],
    );
    let ext = make_file(
        "ext:r-stdlib/dplyr.R",
        vec![make_sym("group_by", SymbolKind::Function)],
        vec![],
    );
    let (id_a, id_b, ext_id) = {
        let (_, id_map) = build_index(&[&a, &b, &ext]);
        (
            sym_id(&id_map, "R/group_by.R", "group_by"),
            sym_id(&id_map, "R/grouped_df.R", "group_by"),
            sym_id(&id_map, "ext:r-stdlib/dplyr.R", "group_by"),
        )
    };
    let res = resolve_call("R/main.R", "group_by", &[&a, &b, &ext])
        .expect("bare R call binds an internal function");
    assert_eq!(res.strategy, "default_namespaceless_global");
    assert_ne!(
        res.target_symbol_id, ext_id,
        "must not bind the ext:r-stdlib stub"
    );
    assert!(
        res.target_symbol_id == id_a || res.target_symbol_id == id_b,
        "binds an internal group_by (got {})",
        res.target_symbol_id
    );
}

#[test]
fn r_external_only_name_stays_unresolved() {
    // `expect_equal` (testthat) is owned ONLY by an external file — no internal
    // definition. The internal-only rung declines, leaving it for external
    // classification.
    let ext = make_file(
        "ext:r-stdlib/testthat.R",
        vec![make_sym("expect_equal", SymbolKind::Function)],
        vec![],
    );
    let res = resolve_call("R/main.R", "expect_equal", &[&ext]);
    assert!(
        res.is_none(),
        "external-only name must not bind to an internal symbol; got: {res:?}"
    );
}
