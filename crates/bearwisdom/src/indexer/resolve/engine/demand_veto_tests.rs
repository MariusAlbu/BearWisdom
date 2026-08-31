// =============================================================================
// engine/demand_veto_tests — which internal homonyms suppress a demand pull
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::ecosystem::EcosystemId;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};

use super::{DemandVeto, FileLanguages};

fn symbol(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.into(),
        qualified_name: qname.into(),
        kind,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn internal_file(path: &str, language: &str, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.into(),
        language: language.into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn id_map_of(files: &[ParsedFile]) -> HashMap<(String, String), i64> {
    let mut id_map = HashMap::new();
    let mut next = 1i64;
    for pf in files {
        for s in &pf.symbols {
            id_map.insert((pf.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    id_map
}

fn tree_of(files: &[ParsedFile]) -> Compilation {
    Compilation::build(files, &id_map_of(files), Arc::new(TypeArena::new()))
}

/// A compilation whose language-visibility relation is snapshot from `active` —
/// the ecosystems that decide which languages co-declare each other.
fn tree_with_ecosystems(files: &[ParsedFile], active: Vec<EcosystemId>) -> Compilation {
    let ctx = ProjectContext {
        active_ecosystems: active,
        ..Default::default()
    };
    Compilation::build_with_context(
        files,
        &id_map_of(files),
        Arc::new(TypeArena::new()),
        Some(&ctx),
        &Default::default(),
    )
}

fn ref_named(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.into(),
        kind,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn languages(files: &[ParsedFile]) -> FileLanguages<'_> {
    files
        .iter()
        .map(|pf| (pf.path.as_str(), pf.language.as_str()))
        .collect()
}

fn profiles() -> FxHashMap<&'static str, &'static LanguageProfile> {
    crate::indexer::resolve::engine::pipeline::_test_build_profiles()
}

/// A METHOD named like the type cannot satisfy `new InvalidOperationException(…)`,
/// so it must not suppress pulling the external CLASS that can.
#[test]
fn internal_method_does_not_veto_instantiates_pull() {
    let files = vec![internal_file(
        "src/ThrowHelper.cs",
        "csharp",
        vec![symbol(
            "InvalidOperationException",
            "ThrowHelper.InvalidOperationException",
            SymbolKind::Method,
        )],
    )];
    let tree = tree_of(&files);
    let langs = languages(&files);
    let profiles = profiles();
    let veto = DemandVeto::new("csharp", &profiles, &langs);

    assert!(
        !veto.vetoes(
            &tree,
            &ref_named("InvalidOperationException", EdgeKind::Instantiates)
        ),
        "a method is not an instantiation target — the pull stays open"
    );
}

/// Same language, kind the edge admits: the internal declaration is what the
/// ref binds, so nothing external needs materializing.
#[test]
fn same_language_kind_compatible_internal_vetoes() {
    let files = vec![internal_file(
        "src/Domain/Result.cs",
        "csharp",
        vec![symbol("Result", "App.Domain.Result", SymbolKind::Class)],
    )];
    let tree = tree_of(&files);
    let langs = languages(&files);
    let profiles = profiles();
    let veto = DemandVeto::new("csharp", &profiles, &langs);

    assert!(
        veto.vetoes(&tree, &ref_named("Result", EdgeKind::Instantiates)),
        "a same-language class the ref can instantiate suppresses the pull"
    );
}

/// A class of another language shares only the spelling — a TypeScript
/// `DateTime` is not what a C# ref names.
#[test]
fn cross_language_internal_does_not_veto() {
    let files = vec![internal_file(
        "src/web/time.ts",
        "typescript",
        vec![symbol("DateTime", "DateTime", SymbolKind::Class)],
    )];
    let tree = tree_of(&files);
    let langs = languages(&files);
    let profiles = profiles();
    let veto = DemandVeto::new("csharp", &profiles, &langs);

    assert!(
        !veto.vetoes(&tree, &ref_named("DateTime", EdgeKind::TypeRef)),
        "a TypeScript class must not suppress the pull for a C# ref"
    );
}

/// One ecosystem co-declares several languages, and a declaration in any of
/// them is what a ref in the others binds — an npm-served project's TypeScript
/// class IS the `ApiClient` a Vue ref names, so the internal definition vetoes.
#[test]
fn co_bound_language_internal_vetoes() {
    let files = vec![internal_file(
        "src/api/client.ts",
        "typescript",
        vec![symbol("ApiClient", "ApiClient", SymbolKind::Class)],
    )];
    let tree = tree_with_ecosystems(&files, vec![EcosystemId::new("npm")]);
    let langs = languages(&files);
    let profiles = profiles();
    let veto = DemandVeto::new("vue", &profiles, &langs);

    assert!(
        veto.vetoes(&tree, &ref_named("ApiClient", EdgeKind::Instantiates)),
        "a co-declared language's internal class is what the ref binds"
    );
}

/// The veto is about INTERNAL definitions only — an external symbol already in
/// the tree never suppresses another package's declaration of the same name.
#[test]
fn external_definition_never_vetoes() {
    let files = vec![internal_file(
        "ext:dotnet:CoreLib/debug.cs",
        "csharp",
        vec![symbol(
            "Assert",
            "System.Diagnostics.Debug.Assert",
            SymbolKind::Class,
        )],
    )];
    let tree = tree_of(&files);
    let langs = languages(&files);
    let profiles = profiles();
    let veto = DemandVeto::new("csharp", &profiles, &langs);

    assert!(
        !veto.vetoes(&tree, &ref_named("Assert", EdgeKind::TypeRef)),
        "an external same-name symbol must not suppress the pull"
    );
}
