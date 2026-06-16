use std::collections::HashMap;

use crate::indexer::resolve::legacy::{build_scope_chain, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;

use super::profile::TEMPL_PROFILE;
use super::TEMPL_HOOKS;

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

fn calls_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "templ".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn build_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
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

/// Drive the generic engine resolver for the file's first ref the way
/// production does for a chain-less ref — through the profile's
/// `kind_compatible_table`, with no per-language `resolve_ref` hook.
fn resolve(source: &ParsedFile, index: &SymbolIndex) -> Option<Resolution> {
    let file_ctx = TEMPL_HOOKS.build_file_context(source, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: index,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&TEMPL_PROFILE)
}

#[test]
fn at_component_binds_same_file_function() {
    // `@Header()` resolves to the same-file `templ Header` function symbol.
    let file = make_file(
        "views.templ",
        vec![
            make_symbol("views", "views", SymbolKind::Class),
            make_symbol("Header", "views.Header", SymbolKind::Function),
        ],
        vec![calls_ref("Header")],
    );
    let (index, id_map) = build_env(&[&file]);
    let res = resolve(&file, &index).expect("same-file component call should resolve");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("views.templ".to_string(), "views.Header".to_string()))
            .unwrap()
    );
}

#[test]
fn calls_edge_rejects_class_kind_target() {
    // The profile's kind table gates a `Calls` edge to function/method/
    // constructor. A same-named `class` symbol is not a valid call target, so
    // the ref stays unresolved rather than binding to it.
    let file = make_file(
        "views.templ",
        vec![
            make_symbol("views", "views", SymbolKind::Class),
            make_symbol("Header", "views.Header", SymbolKind::Class),
        ],
        vec![calls_ref("Header")],
    );
    let (index, _id_map) = build_env(&[&file]);
    assert!(
        resolve(&file, &index).is_none(),
        "a class-kind symbol must not satisfy a Calls edge"
    );
}
