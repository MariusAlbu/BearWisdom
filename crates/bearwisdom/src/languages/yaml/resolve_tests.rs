use super::profile::YAML_PROFILE;
use super::YAML_HOOKS;
use crate::indexer::resolve::engine::{build_scope_chain, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

/// YAML's file-scoped class symbol is named after the full file basename
/// (including extension), e.g. `action.yml` — see `yaml::extract`. The
/// `BasenameWithExt` stem-match rule binds against that name.
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

fn uses_ref(target: &str) -> ExtractedRef {
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
        language: "yaml".to_string(),
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
    let file_ctx = YAML_HOOKS.build_file_context(source, None).unwrap();
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
    .resolve_all_with_profile(&YAML_PROFILE)
}

#[test]
fn reusable_workflow_resolves_verbatim() {
    // `uses: ./reusable.yml` → the file itself.
    let target = make_file(
        "/repo/.github/workflows/reusable.yml",
        vec![make_class_symbol("reusable.yml")],
        vec![],
    );
    let source = make_file(
        "/repo/.github/workflows/ci.yml",
        vec![make_class_symbol("ci.yml")],
        vec![uses_ref("./reusable.yml")],
    );
    let (index, id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("reusable workflow should resolve");
    assert_eq!(res.strategy, "yaml_uses");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&(
                "/repo/.github/workflows/reusable.yml".to_string(),
                "reusable.yml".to_string()
            ))
            .unwrap()
    );
}

#[test]
fn composite_action_resolves_via_action_yml() {
    // `uses: ../actions/setup` → `../actions/setup/action.yml`.
    let target = make_file(
        "/repo/.github/actions/setup/action.yml",
        vec![make_class_symbol("action.yml")],
        vec![],
    );
    let source = make_file(
        "/repo/.github/workflows/ci.yml",
        vec![make_class_symbol("ci.yml")],
        vec![uses_ref("../actions/setup")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("composite action should resolve via action.yml");
    assert_eq!(res.strategy, "yaml_uses");
}

#[test]
fn composite_action_falls_back_to_named_yml() {
    // A helper shipped as `<dir>/<name>.yml` rather than `<dir>/<name>/action.yml`.
    let target = make_file(
        "/repo/.github/workflows/helpers/check.yml",
        vec![make_class_symbol("check.yml")],
        vec![],
    );
    let source = make_file(
        "/repo/.github/workflows/ci.yml",
        vec![make_class_symbol("ci.yml")],
        vec![uses_ref("./helpers/check")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("named .yml fallback should resolve");
    assert_eq!(res.strategy, "yaml_uses");
}

#[test]
fn unmatched_uses_returns_none() {
    let source = make_file(
        "/repo/.github/workflows/ci.yml",
        vec![make_class_symbol("ci.yml")],
        vec![uses_ref("./missing")],
    );
    let (index, _id_map) = build_env(&[&source]);
    assert!(resolve(&source, &index).is_none());
}
