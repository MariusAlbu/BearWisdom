use super::*;
use crate::indexer::resolve::engine::{build_scope_chain, LanguageResolver, RefContext, SymbolIndex};
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
    }
}

fn make_partial_ref(target: &str) -> ExtractedRef {
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

fn make_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "handlebars".to_string(),
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
        .map(|f| make_file(&f.path, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

#[test]
fn relative_partial_in_same_dir_resolves() {
    let target = make_file("themes/casper/header.hbs", vec![make_class_symbol("header")], vec![]);
    let source = make_file(
        "themes/casper/index.hbs",
        vec![make_class_symbol("index")],
        vec![make_partial_ref("header")],
    );
    let (index, id_map) = build_env(&[&source, &target]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("should resolve");
    assert_eq!(res.strategy, "handlebars_partial");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("themes/casper/header.hbs".to_string(), "header".to_string()))
            .unwrap()
    );
}

#[test]
fn nested_partial_via_partials_dir_resolves() {
    let target = make_file(
        "themes/casper/partials/components/header-content.hbs",
        vec![make_class_symbol("header-content")],
        vec![],
    );
    let source = make_file(
        "themes/casper/index.hbs",
        vec![make_class_symbol("index")],
        vec![make_partial_ref("components/header-content")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("should resolve via partials dir");
    assert_eq!(res.strategy, "handlebars_partial");
}

#[test]
fn mustache_underscore_prefix_resolves() {
    // Mustache convention: `{{> footer}}` matches `_footer.mustache` in same dir.
    let target = make_file("templates/_footer.mustache", vec![make_class_symbol("_footer")], vec![]);
    let source = make_file(
        "templates/index.mustache",
        vec![make_class_symbol("index")],
        vec![make_partial_ref("footer")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("should resolve underscore variant");
    assert_eq!(res.strategy, "handlebars_partial");
}

#[test]
fn partial_in_ancestor_partials_dir_resolves() {
    // Source in deep subdir, partial in higher partials/ folder.
    let target = make_file(
        "themes/casper/partials/icons/search.hbs",
        vec![make_class_symbol("search")],
        vec![],
    );
    let source = make_file(
        "themes/casper/post/article.hbs",
        vec![make_class_symbol("article")],
        vec![make_partial_ref("icons/search")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index).expect("should climb to partials/");
    assert_eq!(res.strategy, "handlebars_partial");
}

#[test]
fn unmatched_partial_returns_none() {
    let source = make_file(
        "templates/page.hbs",
        vec![make_class_symbol("page")],
        vec![make_partial_ref("nonexistent")],
    );
    let (index, _id_map) = build_env(&[&source]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    assert!(resolver.resolve(&file_ctx, &ref_ctx, &index).is_none());
}

#[test]
fn camelcase_partial_resolves_to_kebab_case_file() {
    // Ghost email templates: `{{> feedbackButton}}` references `feedback-button.hbs`.
    let target = make_file(
        "ghost/server/email-templates/partials/feedback-button.hbs",
        vec![make_class_symbol("feedback-button")],
        vec![],
    );
    let source = make_file(
        "ghost/server/email-templates/template.hbs",
        vec![make_class_symbol("template")],
        vec![make_partial_ref("feedbackButton")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver
        .resolve(&file_ctx, &ref_ctx, &index)
        .expect("camelCase partial should resolve to kebab-case file");
    assert_eq!(res.strategy, "handlebars_partial");
}

#[test]
fn calls_kind_refs_are_not_resolved_by_partial_resolver() {
    // Helper-call refs (Calls kind) are emitted from the embedded JS path
    // and route through the TS resolver, not this one. Returning Some for
    // Calls would silently bind helper invocations to unrelated files.
    let source = make_file(
        "templates/page.hbs",
        vec![make_class_symbol("page")],
        vec![ExtractedRef {
            source_symbol_index: 0,
            target_name: "eq".to_string(),
            kind: EdgeKind::Calls,
            line: 0,
            module: None,
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }],
    );
    let (index, _id_map) = build_env(&[&source]);
    let resolver = HandlebarsResolver;
    let file_ctx = resolver.build_file_context(&source, None);
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    assert!(resolver.resolve(&file_ctx, &ref_ctx, &index).is_none());
}
