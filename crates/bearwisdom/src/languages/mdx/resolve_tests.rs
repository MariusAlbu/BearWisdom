use super::MdxResolver;
use crate::indexer::resolve::engine::{
    build_scope_chain, LanguageResolver, RefContext, SymbolIndex,
};
use crate::types::*;
use std::collections::HashMap;

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    scope: Option<&str>,
) -> ExtractedSymbol {
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
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_import_ref(source_idx: usize, target: &str, module: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(
    path: &str,
    language: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
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
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
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
        .map(|f| make_file(&f.path, &f.language, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

#[test]
fn jsx_calls_dispatched_to_ts_resolver_for_same_file_export() {
    // The MDX host emits the JSX `Calls` ref against the file's host
    // class symbol, while the synthetic TS `ScriptBlock` region's
    // `export function Button() {}` lands in the same `pf.symbols`
    // (post-splice). The TS resolver's same-file lookup binds them.
    // This proves the dispatcher routes JSX Calls into the TS path —
    // the Markdown resolver alone returns None for non-Imports refs
    // and the test would fail without the new dispatcher.
    let mdx = make_file(
        "docs/page.mdx",
        "mdx",
        vec![
            make_symbol("page", "page", SymbolKind::Class, None),
            make_symbol("Button", "Button", SymbolKind::Function, None),
        ],
        vec![make_ref(0, "Button", EdgeKind::Calls, 5)],
    );
    let (index, id_map) = build_env(&[&mdx]);
    let resolver = MdxResolver;
    let file_ctx = resolver.build_file_context(&mdx, None);
    let ref_ctx = RefContext {
        extracted_ref: &mdx.refs[0],
        source_symbol: &mdx.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("JSX Calls ref must dispatch to TS resolver and bind same-file Button");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("docs/page.mdx".to_string(), "Button".to_string()))
            .unwrap()
    );
}

#[test]
fn markdown_link_imports_route_through_markdown_resolver() {
    // The MDX host extractor emits link Imports refs identical in shape to
    // Markdown's; the dispatcher must route those through the path-based
    // MarkdownResolver, not TypeScriptResolver.
    let target = make_file(
        "docs/api/overview.md",
        "markdown",
        vec![make_symbol(
            "overview",
            "overview",
            SymbolKind::Class,
            None,
        )],
        vec![],
    );
    let mdx = make_file(
        "docs/page.mdx",
        "mdx",
        vec![make_symbol("page", "page", SymbolKind::Class, None)],
        vec![make_ref(0, "api/overview", EdgeKind::Imports, 1)],
    );

    let (index, id_map) = build_env(&[&mdx, &target]);
    let resolver = MdxResolver;
    let file_ctx = resolver.build_file_context(&mdx, None);
    let ref_ctx = RefContext {
        extracted_ref: &mdx.refs[0],
        source_symbol: &mdx.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("relative .md link should resolve via MarkdownResolver");
    assert_eq!(res.strategy, "markdown_relative_link");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("docs/api/overview.md".to_string(), "overview".to_string()))
            .unwrap()
    );
}

#[test]
fn jsx_calls_with_no_matching_import_falls_through_to_unresolved() {
    // No TS import binds the JSX target; the dispatcher must NOT pretend
    // to resolve it. The engine will land it in unresolved_refs.
    let mdx = make_file(
        "docs/page.mdx",
        "mdx",
        vec![make_symbol("page", "page", SymbolKind::Class, None)],
        vec![make_ref(0, "Unrelated", EdgeKind::Calls, 5)],
    );
    let (index, _) = build_env(&[&mdx]);
    let resolver = MdxResolver;
    let file_ctx = resolver.build_file_context(&mdx, None);
    let ref_ctx = RefContext {
        extracted_ref: &mdx.refs[0],
        source_symbol: &mdx.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "no import → no Tier-1 resolution"
    );
}
