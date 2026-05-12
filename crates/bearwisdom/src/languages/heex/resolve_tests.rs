use super::*;
use crate::indexer::resolve::engine::{build_scope_chain, LanguageResolver, RefContext, SymbolIndex};
use crate::types::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_method_symbol(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Method,
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

fn make_calls_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, lang: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: lang.to_string(),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn ext_component_resolves_via_bare_name() {
    // Simulate a Phoenix.Component.form indexed as an external symbol.
    let ext_file = make_file(
        "ext:idx:/deps/phoenix_live_view/lib/phoenix_component.ex",
        "elixir",
        vec![make_method_symbol("form", "Phoenix.Component.form")],
        vec![],
    );
    let heex_file = make_file(
        "lib/web/templates/auth/login.html.heex",
        "heex",
        vec![make_class_symbol("login.html")],
        vec![make_calls_ref("form")],
    );
    let (index, id_map) = build_env(&[&ext_file, &heex_file]);
    let resolver = HeexResolver;
    let file_ctx = resolver.build_file_context(&heex_file, None);
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(res.is_some(), "<.form> should resolve to external Phoenix.Component.form");
    let res = res.unwrap();
    assert_eq!(res.strategy, "heex_ext_component");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "ext:idx:/deps/phoenix_live_view/lib/phoenix_component.ex".to_string(),
                "Phoenix.Component.form".to_string()
            ))
            .unwrap()
    );
}

#[test]
fn internal_component_resolves_when_no_ext_match() {
    // Project-defined component — no ext: path.
    let comp_file = make_file(
        "lib/my_app_web/components/core_components.ex",
        "elixir",
        vec![make_method_symbol("button", "MyAppWeb.CoreComponents.button")],
        vec![],
    );
    let heex_file = make_file(
        "lib/web/templates/page/index.html.heex",
        "heex",
        vec![make_class_symbol("index.html")],
        vec![make_calls_ref("button")],
    );
    let (index, id_map) = build_env(&[&comp_file, &heex_file]);
    let resolver = HeexResolver;
    let file_ctx = resolver.build_file_context(&heex_file, None);
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(res.is_some(), "<.button> should resolve to internal component");
    assert_eq!(res.unwrap().strategy, "heex_internal_component");
    let _ = id_map; // ensure id_map used
}

#[test]
fn dotted_target_skipped_by_resolver() {
    let heex_file = make_file(
        "lib/web/templates/page/index.html.heex",
        "heex",
        vec![make_class_symbol("index.html")],
        vec![make_calls_ref("Phoenix.Component.form")],
    );
    let (index, _id_map) = build_env(&[&heex_file]);
    let resolver = HeexResolver;
    let file_ctx = resolver.build_file_context(&heex_file, None);
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(res.is_none(), "dotted refs should pass through to heuristic");
}

#[test]
fn infer_external_namespace_dotted_phoenix_root() {
    let heex_file = make_file(
        "lib/web/templates/page/index.html.heex",
        "heex",
        vec![make_class_symbol("index.html")],
        vec![make_calls_ref("Phoenix.Component.form")],
    );
    let (_, _) = build_env(&[&heex_file]);
    let resolver = HeexResolver;
    let file_ctx = resolver.build_file_context(&heex_file, None);
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns.as_deref(), Some("Phoenix"));
}
