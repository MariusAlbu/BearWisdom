// =============================================================================
// vue/hooks_tests.rs — VueHooks::build_file_context synthetic-import injection
// =============================================================================

use super::super::global_registry::{VueComponentSource, VueGlobalRegistry};
use super::VueHooks;
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

fn call_ref(target: &str) -> ExtractedRef {
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
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn component_sym() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "Caller".to_string(),
        qualified_name: "Caller".to_string(),
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

fn vue_file(refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: "src/pages/Home.vue".to_string(),
        language: "vue".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 1,
        mtime: None,
        package_id: None,
        content: Some(String::new()),
        has_errors: false,
        symbols: vec![component_sym()],
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

fn ctx_with(registry: VueGlobalRegistry) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    ctx.plugin_state.set(registry);
    ctx
}

fn module_for<'a>(ctx: &'a crate::indexer::resolve::legacy::FileContext, name: &str) -> Option<&'a str> {
    ctx.imports
        .iter()
        .find(|e| e.imported_name == name)
        .and_then(|e| e.module_path.as_deref())
}

#[test]
fn injects_exact_module_for_pascal_tag_from_components_dts() {
    let mut registry = VueGlobalRegistry::default();
    registry.components.insert(
        "HoppStyleButton".to_string(),
        VueComponentSource::AutoImportModule {
            module: "./components/style/Button.vue".to_string(),
        },
    );
    let ctx = ctx_with(registry);
    let file = vue_file(vec![call_ref("HoppStyleButton")]);

    let fc = VueHooks
        .build_file_context(&file, Some(&ctx))
        .expect("vue file context builds");
    assert_eq!(
        module_for(&fc, "HoppStyleButton"),
        Some("./components/style/Button.vue"),
        "PascalCase tag gets a synthetic import with the d.ts-pinned local module"
    );
}

#[test]
fn injects_exact_module_for_camel_composable_from_auto_imports_dts() {
    // Composables are camelCase — the exact-name map binds them despite the
    // PascalCase library-prefix gate not applying.
    let mut registry = VueGlobalRegistry::default();
    registry.components.insert(
        "useThing".to_string(),
        VueComponentSource::AutoImportModule {
            module: "@scope/composables".to_string(),
        },
    );
    let ctx = ctx_with(registry);
    let file = vue_file(vec![call_ref("useThing")]);

    let fc = VueHooks
        .build_file_context(&file, Some(&ctx))
        .expect("vue file context builds");
    assert_eq!(
        module_for(&fc, "useThing"),
        Some("@scope/composables"),
        "camelCase composable gets a synthetic import with the d.ts-pinned package"
    );
}

#[test]
fn no_registry_entry_injects_nothing() {
    // A tag absent from the registry must not gain a synthetic import — it stays
    // unresolved rather than binding to a hallucinated module.
    let ctx = ctx_with(VueGlobalRegistry::default());
    let file = vue_file(vec![call_ref("HoppStyleButton")]);

    let fc = VueHooks
        .build_file_context(&file, Some(&ctx))
        .expect("vue file context builds");
    assert!(
        module_for(&fc, "HoppStyleButton").is_none(),
        "unregistered tag is not injected"
    );
}
