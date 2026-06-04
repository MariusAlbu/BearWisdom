use super::super::profile::GSP_PROFILE;
use super::GspHooks;
use crate::indexer::resolve::engine::{build_scope_chain, RefContext, Resolution, SymbolIndex};
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

fn make_render_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
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
        language: "gsp".to_string(),
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
    let file_ctx = GspHooks.build_file_context(source, None).unwrap();
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
    .resolve_all_with_profile(&GSP_PROFILE)
}

#[test]
fn underscored_partial_in_same_dir_resolves() {
    // `<g:render template="summary">` from `show.gsp` renders `_summary.gsp`
    // in the same directory (extractor already stripped any leading `_`).
    let target = make_file(
        "grails-app/views/book/_summary.gsp",
        vec![make_class_symbol("_summary")],
        vec![],
    );
    let source = make_file(
        "grails-app/views/book/show.gsp",
        vec![make_class_symbol("show")],
        vec![make_render_ref("summary", EdgeKind::Imports)],
    );
    let (index, id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("partial should resolve");
    assert_eq!(res.strategy, "gsp_template");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "grails-app/views/book/_summary.gsp".to_string(),
                "_summary".to_string()
            ))
            .unwrap()
    );
}

#[test]
fn directly_named_template_resolves() {
    // A template file without the `_` partial convention is also a candidate.
    let target = make_file(
        "grails-app/views/book/parts.gsp",
        vec![make_class_symbol("parts")],
        vec![],
    );
    let source = make_file(
        "grails-app/views/book/show.gsp",
        vec![make_class_symbol("show")],
        vec![make_render_ref("parts", EdgeKind::Imports)],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("template should resolve");
    assert_eq!(res.strategy, "gsp_template");
}

#[test]
fn dir_qualified_template_resolves() {
    // `<g:render template="shared/footer">` resolves relative to the source
    // view's directory: `shared/_footer.gsp`.
    let target = make_file(
        "grails-app/views/book/shared/_footer.gsp",
        vec![make_class_symbol("_footer")],
        vec![],
    );
    let source = make_file(
        "grails-app/views/book/show.gsp",
        vec![make_class_symbol("show")],
        vec![make_render_ref("shared/footer", EdgeKind::Imports)],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("dir-qualified partial should resolve");
    assert_eq!(res.strategy, "gsp_template");
}

#[test]
fn absolute_template_path_declines() {
    // `<g:render template="/shared/footer">` is views-root-relative in Grails,
    // but the engine has no views-root anchor, so it declines rather than
    // mis-bind to a coincidentally-placed `_footer.gsp` under the source dir.
    let target = make_file(
        "grails-app/views/book/shared/_footer.gsp",
        vec![make_class_symbol("_footer")],
        vec![],
    );
    let source = make_file(
        "grails-app/views/book/show.gsp",
        vec![make_class_symbol("show")],
        vec![make_render_ref("/shared/footer", EdgeKind::Imports)],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    assert!(
        resolve(&source, &index).is_none(),
        "leading-slash template must decline (no views-root anchor)"
    );
}

#[test]
fn non_imports_ref_is_not_bound_by_template_strategy() {
    // Only the `<g:render template>` Imports ref binds via the template
    // strategy; a Calls ref to the same name does not bind to a `.gsp` file.
    let target = make_file(
        "grails-app/views/book/_summary.gsp",
        vec![make_class_symbol("_summary")],
        vec![],
    );
    let source = make_file(
        "grails-app/views/book/show.gsp",
        vec![make_class_symbol("show")],
        vec![make_render_ref("summary", EdgeKind::Calls)],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index);
    assert!(res.map_or(true, |r| r.strategy != "gsp_template"));
}

#[test]
fn unmatched_template_stays_unresolved() {
    // No coincidental bind: a template with no matching `.gsp` file in scope
    // resolves to nothing rather than grep-matching a same-named symbol.
    let source = make_file(
        "grails-app/views/book/show.gsp",
        vec![make_class_symbol("show")],
        vec![make_render_ref("nonexistent", EdgeKind::Imports)],
    );
    let (index, _id_map) = build_env(&[&source]);
    assert!(resolve(&source, &index).is_none());
}
