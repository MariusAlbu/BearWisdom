use super::hooks::VbaHooks;
use super::profile::VBA_PROFILE;
use crate::indexer::resolve::legacy::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

/// Run a chain-less ref through the generic engine ladder with VBA's profile —
/// the production path now that `VbaHooks::resolve_ref` is gone. The profile's
/// `name_normalization` carries VBA's case-insensitivity, so a reference
/// written in a different casing binds in the bare-name strategies.
fn resolve_engine(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    index: &SymbolIndex,
) -> Option<Resolution> {
    DefaultResolver {
        file_ctx,
        ref_ctx,
        lookup: index,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&VBA_PROFILE)
}

fn make_symbol(name: &str, qname: &str, kind: SymbolKind, scope: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope.map(|s| s.to_string()),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 1,
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
        language: "vba".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
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

fn build_test_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files
        .iter()
        .map(|f| make_file(&f.path, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

// ---------------------------------------------------------------------------
// Case-insensitive same-file resolution — the behaviour the deleted hook
// provided, now served by the profile's `name_normalization`.
// ---------------------------------------------------------------------------

#[test]
fn test_case_insensitive_same_file_resolution() {
    // The callee `helper` (lowercase) must bind to the same-file `Helper`
    // (capitalized) — VBA identifiers are case-insensitive.
    let file = make_file(
        "Module1.bas",
        vec![
            make_symbol("Main", "Main", SymbolKind::Function, None),
            make_symbol("Helper", "Helper", SymbolKind::Function, None),
        ],
        vec![make_ref(0, "helper", EdgeKind::Calls)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let file_ctx = VbaHooks.build_file_context(&file, None).unwrap();

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let res = resolve_engine(&file_ctx, &ref_ctx, &index)
        .expect("lowercase callee should bind to the capitalized same-file sibling");
    assert_eq!(res.strategy, "default_same_file");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("Module1.bas".to_string(), "Helper".to_string()))
            .unwrap()
    );
}

#[test]
fn test_case_exact_same_file_still_resolves() {
    // A byte-exact reference resolves identically — the normalized comparison
    // is a superset of the exact one.
    let file = make_file(
        "Module1.bas",
        vec![
            make_symbol("Main", "Main", SymbolKind::Function, None),
            make_symbol("DoWork", "DoWork", SymbolKind::Function, None),
        ],
        vec![make_ref(0, "DoWork", EdgeKind::Calls)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let file_ctx = VbaHooks.build_file_context(&file, None).unwrap();

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let res = resolve_engine(&file_ctx, &ref_ctx, &index).expect("exact callee should resolve");
    assert_eq!(res.strategy, "default_same_file");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("Module1.bas".to_string(), "DoWork".to_string()))
            .unwrap()
    );
}

#[test]
fn test_unknown_callee_falls_through() {
    let file = make_file(
        "Module1.bas",
        vec![make_symbol("Main", "Main", SymbolKind::Function, None)],
        vec![make_ref(0, "Nonexistent", EdgeKind::Calls)],
    );

    let (index, _) = build_test_env(&[&file]);
    let file_ctx = VbaHooks.build_file_context(&file, None).unwrap();

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    assert!(
        resolve_engine(&file_ctx, &ref_ctx, &index).is_none(),
        "an unknown callee must stay unresolved"
    );
}
