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
pub mod connectors;
pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::indexer::resolve::engine::{FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup};
use crate::indexer::project_context::ProjectContext;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

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
        crate::languages::typescript::resolve::TypeScriptResolver
            .build_file_context(file, project_ctx)
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
        vec![
            Box::new(connectors::VueGraphQlConnector),
        ]
    }
}
