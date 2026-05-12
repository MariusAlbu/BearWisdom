use super::*;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex,
};
use crate::types::*;
use std::collections::HashMap;

fn make_pug_file(path: &str, host_name: &str, refs: Vec<ExtractedRef>) -> ParsedFile {
    let host = ExtractedSymbol {
        name: host_name.to_string(),
        qualified_name: host_name.to_string(),
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
    };
    ParsedFile {
        path: path.to_string(),
        language: "pug".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 1,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: vec![host],
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

fn import_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn build_index_and_resolve(files: &[&ParsedFile], importer: &ParsedFile) -> Option<crate::indexer::resolve::engine::Resolution> {
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
        .map(|f| ParsedFile {
            path: f.path.clone(),
            language: f.language.clone(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: f.symbols.clone(),
            refs: f.refs.clone(),
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
            flow: crate::types::FlowMeta::default(),
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    let resolver = PugResolver;
    let file_ctx = resolver.build_file_context(importer, None);
    let r = importer.refs.first()?;
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &importer.symbols[0],
        scope_chain: build_scope_chain(importer.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };
    resolver.resolve(&file_ctx, &ref_ctx, &index)
}

#[test]
fn include_relative_subdirectory_resolves() {
    let block = make_pug_file("views/includes/block.pug", "block", vec![]);
    let blockchain = make_pug_file(
        "views/blockchain.pug",
        "blockchain",
        vec![import_ref("includes/block")],
    );
    let res = build_index_and_resolve(&[&block, &blockchain], &blockchain)
        .expect("includes/block should resolve to views/includes/block.pug");
    assert_eq!(res.strategy, "pug_template_include");
}

#[test]
fn extends_sibling_layout_resolves() {
    let layout = make_pug_file("views/layout.pug", "layout", vec![]);
    let page = make_pug_file(
        "views/page.pug",
        "page",
        vec![import_ref("layout")],
    );
    let res = build_index_and_resolve(&[&layout, &page], &page)
        .expect("layout should resolve to views/layout.pug");
    assert_eq!(res.strategy, "pug_template_include");
}

#[test]
fn include_with_parent_dir_resolves() {
    let header = make_pug_file("shared/header.pug", "header", vec![]);
    let page = make_pug_file(
        "views/page.pug",
        "page",
        vec![import_ref("../shared/header")],
    );
    let res = build_index_and_resolve(&[&header, &page], &page)
        .expect("../shared/header should resolve");
    assert_eq!(res.strategy, "pug_template_include");
}

#[test]
fn missing_target_returns_none() {
    let page = make_pug_file(
        "views/page.pug",
        "page",
        vec![import_ref("nope/missing")],
    );
    assert!(build_index_and_resolve(&[&page], &page).is_none());
}
