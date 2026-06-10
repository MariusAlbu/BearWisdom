use super::hooks::HeexHooks;
use crate::indexer::resolve::engine::FileContext;
use crate::indexer::resolve::engine::{build_scope_chain, RefContext, SymbolIndex};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_calls_ref(target: &str) -> ExtractedRef {
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

fn make_file(
    path: &str,
    lang: &str,
    syms: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
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
        .map(|f| make_file(&f.path, &f.language, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn ext_component_not_grep_resolved() {
    // A `<.form>` reference no longer binds to an external symbol merely because
    // one named `form` exists somewhere. The import->component link (CORPUS-1)
    // isn't modeled for HEEx, so the bare component stays unresolved rather than
    // grep-binding to Phoenix.Component.form. The old `heex_ext_component`
    // whole-program by-name fallback was removed (scope-directed, never grep).
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
    let file_ctx = HeexHooks.build_file_context(&heex_file, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = HeexHooks.resolve_ref(&file_ctx, &ref_ctx, &index);
    assert!(
        res.is_none(),
        "bare external component is no longer grep-resolved"
    );
    let _ = id_map;
}

#[test]
fn internal_component_not_grep_resolved() {
    // A project-defined component referenced as `<.button>` has no scope-directed
    // binding (the import->component link isn't modeled for HEEx), so it stays
    // unresolved rather than binding to a same-named symbol by coincidence. The
    // old whole-program by-name fallback (`heex_internal_component`) was removed.
    let comp_file = make_file(
        "lib/my_app_web/components/core_components.ex",
        "elixir",
        vec![make_method_symbol(
            "button",
            "MyAppWeb.CoreComponents.button",
        )],
        vec![],
    );
    let heex_file = make_file(
        "lib/web/templates/page/index.html.heex",
        "heex",
        vec![make_class_symbol("index.html")],
        vec![make_calls_ref("button")],
    );
    let (index, id_map) = build_env(&[&comp_file, &heex_file]);
    let file_ctx = HeexHooks.build_file_context(&heex_file, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = HeexHooks.resolve_ref(&file_ctx, &ref_ctx, &index);
    assert!(
        res.is_none(),
        "bare internal component is no longer grep-resolved"
    );
    let _ = id_map;
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
    let file_ctx = HeexHooks.build_file_context(&heex_file, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let res = HeexHooks.resolve_ref(&file_ctx, &ref_ctx, &index);
    assert!(
        res.is_none(),
        "dotted refs should pass through to heuristic"
    );
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
    let file_ctx = HeexHooks.build_file_context(&heex_file, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &heex_file.refs[0],
        source_symbol: &heex_file.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        use std::collections::HashMap;
        let empty_lookup =
            crate::indexer::resolve::engine::SymbolIndex::build(&[], &HashMap::new());
        crate::languages::heex::hooks::HeexHooks.classify_external(
            &ref_ctx,
            &file_ctx,
            None,
            &empty_lookup,
        )
    };
    assert_eq!(ns.as_deref(), Some("Phoenix"));
}
