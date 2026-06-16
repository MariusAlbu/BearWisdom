use super::_test_colocated_view_file as colocated_view_file;
use super::HeexHooks;
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::resolve::legacy::{RefContext, SymbolIndex};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::HashMap;

fn ctx_with_mix_deps(deps: &[&str]) -> crate::indexer::project_context::ProjectContext {
    let mut ctx = crate::indexer::project_context::ProjectContext::default();
    let mut m = ManifestData::default();
    for d in deps {
        m.dependencies.insert((*d).to_string());
    }
    ctx.manifests.insert(ManifestKind::Mix, m);
    ctx
}

fn dotted_ref(target: &str) -> ExtractedRef {
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

fn source_sym() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "tpl".to_string(),
        qualified_name: "tpl".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
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

fn classify(
    target: &str,
    project_ctx: Option<&crate::indexer::project_context::ProjectContext>,
) -> Option<String> {
    let r = dotted_ref(target);
    let sym = source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let file_ctx = HeexHooks
        .build_file_context(
            &crate::types::ParsedFile {
                path: "lib/web/templates/page/index.html.heex".to_string(),
                language: "heex".to_string(),
                content_hash: String::new(),
                size: 0,
                line_count: 1,
                mtime: None,
                package_id: None,
                content: None,
                has_errors: false,
                symbols: vec![],
                refs: vec![],
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
            },
            None,
        )
        .unwrap();
    let index = SymbolIndex::build(&[], &HashMap::new());
    HeexHooks.classify_external(&ref_ctx, &file_ctx, project_ctx, &index)
}

#[test]
fn phoenix_root_external_when_mix_declares_phoenix() {
    // `.heex` ref to `Phoenix.Endpoint` classifies external because mix.exs
    // declares the phoenix Hex dependency — same manifest path the elixir
    // hook uses.
    let ctx = ctx_with_mix_deps(&["phoenix", "ecto"]);
    assert_eq!(classify("Phoenix.Endpoint", Some(&ctx)).as_deref(), Some("Phoenix"));
}

#[test]
fn phoenix_root_not_external_without_manifest() {
    // No mix.exs declaring phoenix → a Hex-package root is honestly
    // unresolved, not short-circuited to external.
    assert_eq!(classify("Phoenix.Endpoint", None), None);
    let empty = ctx_with_mix_deps(&[]);
    assert_eq!(classify("Phoenix.Endpoint", Some(&empty)), None);
}

#[test]
fn stdlib_root_external_regardless_of_manifest() {
    // `Enum` is in the stdlib subset — external with or without a manifest.
    assert_eq!(classify("Enum.map", None).as_deref(), Some("Enum"));
    let ctx = ctx_with_mix_deps(&[]);
    assert_eq!(classify("Enum.map", Some(&ctx)).as_deref(), Some("Enum"));
}

#[test]
fn single_level_context_maps_to_view() {
    assert_eq!(
        colocated_view_file("lib/plausible_web/templates/sso/login_form.html.heex").as_deref(),
        Some("lib/plausible_web/views/sso_view.ex"),
    );
}

#[test]
fn nested_context_mirrors_path_and_names_deepest_dir() {
    // `templates/admin/episode/edit.html.heex` is rendered by
    // `Admin.EpisodeView` at `views/admin/episode_view.ex` — the deepest
    // directory names the view, intervening dirs are mirrored.
    assert_eq!(
        colocated_view_file("lib/changelog_web/templates/admin/episode/edit.html.heex").as_deref(),
        Some("lib/changelog_web/views/admin/episode_view.ex"),
    );
}

#[test]
fn deeply_nested_context_preserves_all_parents() {
    assert_eq!(
        colocated_view_file("lib/web/templates/a/b/c/page.html.heex").as_deref(),
        Some("lib/web/views/a/b/c_view.ex"),
    );
}

#[test]
fn template_directly_under_templates_has_no_view() {
    // No context directory to name a view after.
    assert_eq!(
        colocated_view_file("lib/web/templates/page.html.heex"),
        None,
    );
}

#[test]
fn path_without_templates_segment_is_none() {
    assert_eq!(colocated_view_file("lib/web/live/foo_live.html.heex"), None);
}

#[test]
fn backslash_paths_are_normalized() {
    assert_eq!(
        colocated_view_file(r"lib\changelog_web\templates\admin\episode\index.html.heex")
            .as_deref(),
        Some("lib/changelog_web/views/admin/episode_view.ex"),
    );
}
