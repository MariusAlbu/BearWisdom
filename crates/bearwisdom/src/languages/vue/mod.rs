//! Vue language plugin.
//!
//! Vue Single File Components (.vue) are parsed using the HTML grammar as a
//! structural fallback. There is no tree-sitter-vue in the workspace yet.
//!
//! What we extract at this grammar level:
//! - The file itself → a Class symbol (component name inferred from filename)
//! - PascalCase tags in the template → Calls edges (component usages)
//! - Kebab-case custom element tags (contains at least one hyphen) → Calls edges
//!   (normalized to PascalCase)
//! - Event handler directives (@event / v-on) → Calls edges to the handler method
//!
//! The <script> block's JS/TS symbols are handled by the JS/TS extractor when
//! the indexer processes the embedded text as a separate extraction target.

pub(crate) mod predicates;
pub(crate) mod type_checker;
pub mod connectors;
pub mod extract;
pub mod global_registry;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup};
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult, ParsedFile};

// ---------------------------------------------------------------------------
// VueResolver — wraps TypeScriptResolver, claims "vue" as its language_id.
//
// Vue SFC template refs (language = "vue") go through this resolver instead
// of falling through with no resolver at all.  All logic delegates to
// TypeScriptResolver; the builtin check in `infer_external_namespace` fires
// for `.vue` files automatically via the `file_ctx.file_path` check.
// ---------------------------------------------------------------------------
pub(crate) struct VueResolver;

impl LanguageResolver for VueResolver {
    fn language_ids(&self) -> &[&str] {
        &["vue"]
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut ctx = crate::languages::typescript::resolve::TypeScriptResolver
            .build_file_context(file, project_ctx);

        // Inject synthetic import entries for globally-registered Vue components.
        //
        // For each Calls ref whose target is PascalCase and doesn't already
        // appear in the file's import list, check the project-wide global
        // registry stored in plugin_state.  If a library covers that component
        // (via a prefix convention), add a synthetic ImportEntry so the TS
        // resolver's existing import loop resolves `ComponentName` →
        // `package.ComponentName` against the external index.
        if let Some(ctx_ref) = project_ctx {
            if let Some(registry) = ctx_ref.plugin_state.get::<global_registry::VueGlobalRegistry>() {
                if !registry.is_empty() {
                    // Collect component names referenced by this file via Calls edges.
                    // We only process refs with no module (i.e., not already imported).
                    let already_imported: std::collections::HashSet<&str> =
                        ctx.imports.iter().map(|e| e.imported_name.as_str()).collect();

                    let mut extra_imports: Vec<ImportEntry> = Vec::new();
                    for r in &file.refs {
                        let name = &r.target_name;
                        // Only PascalCase names (component references)
                        if !name.chars().next().map_or(false, |c| c.is_uppercase()) {
                            continue;
                        }
                        // Already imported — skip
                        if already_imported.contains(name.as_str()) {
                            continue;
                        }
                        // Avoid duplicates within the extra list
                        if extra_imports.iter().any(|e| &e.imported_name == name) {
                            continue;
                        }
                        // Check global registry for a library match
                        if let Some(pkg) = global_registry::library_for_name(registry, name) {
                            extra_imports.push(ImportEntry {
                                imported_name: name.clone(),
                                module_path: Some(pkg.to_string()),
                                alias: None,
                                is_wildcard: false,
                            });
                        }
                        // Check explicit single-component registrations — inject a
                        // wildcard entry so the by-name heuristic can find the symbol.
                        // We don't know the exact file path at this point, but we
                        // can mark the component as "global" so it's not classified
                        // as external.  The heuristic resolver will find it via
                        // `by_name` if it's indexed.
                        // (No action needed here — the heuristic already falls back
                        // to by-name lookup; the entry in the registry is enough to
                        // prevent external classification via `infer_external_namespace`.)
                    }
                    if !extra_imports.is_empty() {
                        ctx.imports.extend(extra_imports);
                    }
                }
            }
        }

        ctx
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        crate::languages::typescript::resolve::TypeScriptResolver
            .resolve(file_ctx, ref_ctx, lookup)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // If this component is globally registered (explicitly via app.component),
        // suppress external classification so the heuristic resolver can find
        // it by name in the project index.
        if let Some(ctx_ref) = project_ctx {
            if let Some(registry) = ctx_ref.plugin_state.get::<global_registry::VueGlobalRegistry>() {
                let name = &ref_ctx.extracted_ref.target_name;
                if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    if let Some(global_registry::VueComponentSource::ExplicitRegistration { .. }) =
                        registry.components.get(name.as_str())
                    {
                        return None;
                    }
                }
            }
        }
        crate::languages::typescript::resolve::TypeScriptResolver
            .infer_external_namespace(file_ctx, ref_ctx, project_ctx)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        crate::languages::typescript::resolve::TypeScriptResolver
            .infer_external_namespace_with_lookup(file_ctx, ref_ctx, project_ctx, lookup)
    }
}

pub struct VuePlugin;

impl LanguagePlugin for VuePlugin {
    fn id(&self) -> &str {
        "vue"
    }

    fn language_ids(&self) -> &[&str] {
        &["vue"]
    }

    fn extensions(&self) -> &[&str] {
        &[".vue"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // HTML grammar handles the SFC outer shell (template/script/style tags).
        Some(tree_sitter_html::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn extract_connection_points(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_vue_graphql_points(source)
    }

    /// Split out `<script>` / `<script setup lang="ts">` and `<style>` blocks
    /// for sub-extraction by the JS/TS/CSS/SCSS plugins. Indexer splices the
    /// resulting symbols/refs back into the same `.vue` file.
    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        crate::languages::common::extract_html_script_style_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Vue SFC: symbols come from script block; at the grammar level only
        // the component-level element is meaningful.
        &["script_element", "template_element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        // Component invocations and event handler directives.
        &["element", "self_closing_tag", "directive_attribute"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        crate::languages::typescript::keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(VueResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![]
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::VueChecker))
    }

    fn populate_project_state(
        &self,
        state: &mut PluginStateBag,
        parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &ProjectContext,
    ) {
        let parsed_paths: Vec<String> = parsed
            .iter()
            .filter(|pf| !pf.path.starts_with("ext:"))
            .map(|pf| pf.path.clone())
            .collect();
        let registry = global_registry::scan_global_registrations(project_root, &parsed_paths);
        if !registry.is_empty() {
            tracing::info!(
                "Vue global registry: {} components/prefixes, unplugin_auto_import={}",
                registry.components.len(),
                registry.has_unplugin_auto_import,
            );
        }
        state.set(registry);
    }
}
