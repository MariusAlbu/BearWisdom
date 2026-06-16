use std::collections::HashMap;

use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{build_scope_chain, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;

use super::hooks::infer_ansible_external;
use super::profile::JINJA_PROFILE;
use super::JINJA_HOOKS;

fn ctx_with_roles(role_names: &[&str]) -> ProjectContext {
    let mut deps = std::collections::HashSet::new();
    for &name in role_names {
        deps.insert(name.to_string());
    }
    let data = ManifestData {
        dependencies: deps,
        ..Default::default()
    };
    let mut manifests = HashMap::new();
    manifests.insert(ManifestKind::AnsibleRequirements, data);
    ProjectContext {
        manifests,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// infer_ansible_external
// ---------------------------------------------------------------------------

#[test]
fn jinja_resolver_routes_declared_external_role_prefix_to_external() {
    let ctx = ctx_with_roles(&["systemd_docker_base"]);
    let ns = infer_ansible_external("systemd_docker_base_docker_service_name", Some(&ctx));
    assert_eq!(ns.as_deref(), Some("ansible.systemd_docker_base"));
}

#[test]
fn jinja_resolver_exact_role_name_match_is_external() {
    let ctx = ctx_with_roles(&["traefik"]);
    let ns = infer_ansible_external("traefik", Some(&ctx));
    assert_eq!(ns.as_deref(), Some("ansible.traefik"));
}

#[test]
fn jinja_resolver_no_false_positive_on_prefix_without_underscore() {
    // `docker` role must NOT match `dockerfile_path` — the name segment boundary
    // is enforced by requiring `<role>_`.
    let ctx = ctx_with_roles(&["docker"]);
    let ns = infer_ansible_external("dockerfile_path", Some(&ctx));
    // `dockerfile_path` does NOT start with `docker_`, so no match.
    assert!(ns.is_none());
}

#[test]
fn jinja_resolver_local_role_var_returns_none() {
    // A var whose role IS declared externally but the var prefix actually matches
    // the local role name — this should still be classified external.
    let ctx = ctx_with_roles(&["systemd_docker_base", "traefik"]);
    let ns = infer_ansible_external("traefik_enabled", Some(&ctx));
    assert_eq!(ns.as_deref(), Some("ansible.traefik"));
}

#[test]
fn jinja_resolver_no_match_returns_none() {
    let ctx = ctx_with_roles(&["systemd_docker_base"]);
    let ns = infer_ansible_external("matrix_base_enabled", Some(&ctx));
    assert!(ns.is_none());
}

#[test]
fn jinja_resolver_no_project_ctx_returns_none() {
    let ns = infer_ansible_external("systemd_docker_base_enabled", None);
    assert!(ns.is_none());
}

#[test]
fn jinja_resolver_empty_manifest_returns_none() {
    let ctx = ctx_with_roles(&[]);
    let ns = infer_ansible_external("anything", Some(&ctx));
    assert!(ns.is_none());
}

// ---------------------------------------------------------------------------
// Template-path resolution — driven by the engine's import-path strategy
// (profile.import_resolution = JINJA_IMPORTS), no per-language resolve_ref.
// ---------------------------------------------------------------------------

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
        language: "jinja".to_string(),
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

/// Drive the generic engine resolver for a chain-less `Imports` ref exactly as
/// production does: the profile's `import_resolution` makes
/// `resolve_via_import_path` the first ladder rung. No `resolve_ref` hook.
fn resolve(source: &ParsedFile, index: &SymbolIndex) -> Option<Resolution> {
    let file_ctx = JINJA_HOOKS.build_file_context(source, None).unwrap();
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
    .resolve_all_with_profile(&JINJA_PROFILE)
}

#[test]
fn extends_binds_template_path_to_file_stem_class() {
    // `{% extends "base.j2" %}` — the extractor strips the extension, so the
    // target is `base`; the candidate re-appends `.j2` and binds the
    // referenced template's file-stem class regardless of its name.
    let target = make_file("templates/base.j2", vec![make_class_symbol("base")], vec![]);
    let source = make_file(
        "templates/page.j2",
        vec![make_class_symbol("page")],
        vec![import_ref("base")],
    );
    let (index, id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("template path should resolve via the engine");
    assert_eq!(res.strategy, "jinja_template_path");
    assert_eq!(res.confidence, 1.0);
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("templates/base.j2".to_string(), "base".to_string()))
            .unwrap()
    );
}

#[test]
fn include_ascends_with_dotdot_across_extensions() {
    // `{% include "../partials/header" %}` from `templates/admin/page.j2`,
    // partial authored as `.jinja` — candidate generation tries each extension.
    let target = make_file(
        "templates/partials/header.jinja",
        vec![make_class_symbol("header")],
        vec![],
    );
    let source = make_file(
        "templates/admin/page.j2",
        vec![make_class_symbol("page")],
        vec![import_ref("../partials/header")],
    );
    let (index, _id_map) = build_env(&[&source, &target]);
    let res = resolve(&source, &index).expect("dotdot ascent should resolve via the engine");
    assert_eq!(res.strategy, "jinja_template_path");
}

#[test]
fn unmatched_template_path_returns_none() {
    let source = make_file(
        "templates/page.j2",
        vec![make_class_symbol("page")],
        vec![import_ref("missing")],
    );
    let (index, _id_map) = build_env(&[&source]);
    assert!(resolve(&source, &index).is_none());
}
