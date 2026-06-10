use super::profile::MARKDOWN_PROFILE;
use super::MARKDOWN_HOOKS;
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

fn link_ref(target: &str) -> ExtractedRef {
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
        language: "markdown".to_string(),
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
    let file_ctx = MARKDOWN_HOOKS.build_file_context(source, None).unwrap();
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
    .resolve_all_with_profile(&MARKDOWN_PROFILE)
}

#[test]
fn relative_link_appends_md_extension() {
    // `[overview](./overview)` → `docs/overview.md`.
    let target = make_file(
        "docs/overview.md",
        vec![make_class_symbol("overview")],
        vec![],
    );
    let source = make_file(
        "docs/index.md",
        vec![make_class_symbol("index")],
        vec![link_ref("./overview")],
    );
    let (index, id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("appended .md should resolve");
    assert_eq!(res.strategy, "markdown_relative_link");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("docs/overview.md".to_string(), "overview".to_string()))
            .unwrap()
    );
}

#[test]
fn relative_link_with_extension_resolves_verbatim() {
    // `[changelog](./CHANGELOG.md)` binds the file directly without re-extending.
    let target = make_file(
        "docs/CHANGELOG.md",
        vec![make_class_symbol("CHANGELOG")],
        vec![],
    );
    let source = make_file(
        "docs/index.md",
        vec![make_class_symbol("index")],
        vec![link_ref("./CHANGELOG.md")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("verbatim .md should resolve");
    assert_eq!(res.strategy, "markdown_relative_link");
}

#[test]
fn directory_link_resolves_via_index_entry() {
    // `[guide](./guide)` where `guide/` holds `index.md`.
    let target = make_file(
        "docs/guide/index.md",
        vec![make_class_symbol("index")],
        vec![],
    );
    let source = make_file(
        "docs/home.md",
        vec![make_class_symbol("home")],
        vec![link_ref("./guide")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("directory index entry should resolve");
    assert_eq!(res.strategy, "markdown_relative_link");
}

#[test]
fn directory_link_resolves_via_readme_entry() {
    let target = make_file(
        "docs/api/README.md",
        vec![make_class_symbol("README")],
        vec![],
    );
    let source = make_file(
        "docs/home.md",
        vec![make_class_symbol("home")],
        vec![link_ref("./api")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("directory README entry should resolve");
    assert_eq!(res.strategy, "markdown_relative_link");
}

#[test]
fn translation_suffix_appends_md_not_replaces() {
    // `[fr](./eslint_prettier.french)` targets the on-disk
    // `eslint_prettier.french.md`. `.french` is a translation tag, not a
    // markdown extension, so `.md` is APPENDED — the candidate
    // `eslint_prettier.french.md` is probed, never `eslint_prettier.md`.
    let target = make_file(
        "docs/eslint_prettier.french.md",
        vec![make_class_symbol("eslint_prettier.french")],
        vec![],
    );
    let source = make_file(
        "docs/index.md",
        vec![make_class_symbol("index")],
        vec![link_ref("./eslint_prettier.french")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("translation-suffix file should resolve");
    assert_eq!(res.strategy, "markdown_relative_link");
}

#[test]
fn link_ascends_with_dotdot() {
    let target = make_file(
        "docs/shared/glossary.md",
        vec![make_class_symbol("glossary")],
        vec![],
    );
    let source = make_file(
        "docs/guide/intro.md",
        vec![make_class_symbol("intro")],
        vec![link_ref("../shared/glossary")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("dotdot ascent should resolve");
    assert_eq!(res.strategy, "markdown_relative_link");
}

#[test]
fn unmatched_link_returns_none() {
    let source = make_file(
        "docs/index.md",
        vec![make_class_symbol("index")],
        vec![link_ref("./missing")],
    );
    let (index, _id_map) = build_env(&[&source]);
    assert!(resolve(&source, &index).is_none());
}
