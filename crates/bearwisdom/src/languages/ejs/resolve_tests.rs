use super::profile::EJS_PROFILE;
use super::EJS_HOOKS;
use crate::indexer::resolve::legacy::{build_scope_chain, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

fn make_class_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Class,
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

fn import_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
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
        language: "ejs".to_string(),
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
        flow: crate::types::FlowMeta::default(),
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

fn resolve(source: &ParsedFile, index: &SymbolIndex) -> Option<Resolution> {
    let file_ctx = EJS_HOOKS.build_file_context(source, None).unwrap();
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
    .resolve_all_with_profile(&EJS_PROFILE)
}

#[test]
fn include_appends_ejs_extension() {
    // `include('./partials/header')` → `views/partials/header.ejs`.
    let target = make_file(
        "views/partials/header.ejs",
        vec![make_class_symbol("header")],
        vec![],
    );
    let source = make_file(
        "views/index.ejs",
        vec![make_class_symbol("index")],
        vec![import_ref("./partials/header")],
    );
    let (index, id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("appended .ejs should resolve");
    assert_eq!(res.strategy, "ejs_partial");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "views/partials/header.ejs".to_string(),
                "header".to_string()
            ))
            .unwrap()
    );
}

#[test]
fn include_explicit_ejs_extension_resolves() {
    // A target already carrying `.ejs` binds verbatim.
    let target = make_file(
        "views/layout.ejs",
        vec![make_class_symbol("layout")],
        vec![],
    );
    let source = make_file(
        "views/index.ejs",
        vec![make_class_symbol("index")],
        vec![import_ref("./layout.ejs")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("verbatim .ejs should resolve");
    assert_eq!(res.strategy, "ejs_partial");
}

#[test]
fn include_ascends_with_dotdot() {
    // `include('../partials/header')` from `views/admin/page.ejs`.
    let target = make_file(
        "views/partials/header.ejs",
        vec![make_class_symbol("header")],
        vec![],
    );
    let source = make_file(
        "views/admin/page.ejs",
        vec![make_class_symbol("page")],
        vec![import_ref("../partials/header")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("dotdot ascent should resolve");
    assert_eq!(res.strategy, "ejs_partial");
}

#[test]
fn include_html_extension_resolves() {
    let target = make_file(
        "views/banner.html",
        vec![make_class_symbol("banner")],
        vec![],
    );
    let source = make_file(
        "views/index.ejs",
        vec![make_class_symbol("index")],
        vec![import_ref("./banner")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("appended .html should resolve");
    assert_eq!(res.strategy, "ejs_partial");
}

#[test]
fn unmatched_include_returns_none() {
    let source = make_file(
        "views/index.ejs",
        vec![make_class_symbol("index")],
        vec![import_ref("./missing")],
    );
    let (index, _id_map) = build_env(&[&source]);
    assert!(resolve(&source, &index).is_none());
}
