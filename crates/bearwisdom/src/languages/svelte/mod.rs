//! Svelte language plugin.
//!
//! Svelte Single File Components (.svelte) are parsed using the HTML grammar as a
//! structural fallback — the same approach used for Vue. There is no
//! tree-sitter-svelte in the workspace.
//!
//! What we extract at this grammar level:
//! - The file itself → a Class symbol (component name inferred from filename)
//! - PascalCase tags in the template → Calls edges (component usages)
//! - Kebab-case custom element tags (contains at least one hyphen) → Calls edges
//!   (normalised to PascalCase)
//! - `on:event` handler directives → Calls edges to the handler function
//! - `{#if}` / `{#each}` block markers → recorded as Calls to their identifiers
//!   when a function reference is present (e.g., `{#each items as item}` → Calls "items")
//!
//! The `<script>` block's JS/TS symbols are handled by the JS/TS extractor when
//! the indexer processes the embedded text as a separate extraction target.

pub(crate) mod predicates;
pub mod connectors;
pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

// ---------------------------------------------------------------------------
// SvelteResolver — wraps TypeScriptResolver, claims "svelte" as its language_id.
//
// Svelte SFC template refs (language = "svelte") need a resolver registered
// under "svelte" in the engine's resolver map. TypeScriptResolver itself only
// declares `["typescript", "javascript", "tsx", "jsx"]` so it never picked up
// .svelte files — those refs fell through with no Tier-1 resolver, lost
// access to file_ctx.imports, and could only attempt the heuristic (which
// has no per-file import context). The result was thousands of unresolved
// `<Button>` template refs in immich-style projects where Button is imported
// in `<script lang="ts">` from a workspace UI package.
//
// Mirrors VueResolver's approach exactly. All logic delegates to
// TypeScriptResolver since Svelte's `<script>` block IS TypeScript.
// ---------------------------------------------------------------------------
pub(crate) struct SvelteResolver;

impl LanguageResolver for SvelteResolver {
    fn language_ids(&self) -> &[&str] {
        &["svelte"]
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

pub struct SveltePlugin;

impl LanguagePlugin for SveltePlugin {
    fn id(&self) -> &str {
        "svelte"
    }

    fn language_ids(&self) -> &[&str] {
        &["svelte"]
    }

    fn extensions(&self) -> &[&str] {
        &[".svelte"]
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
        connectors::extract_svelte_graphql_points(source)
    }

    /// Split out `<script>` and `<style>` blocks for sub-extraction by the
    /// JS/TS/CSS/SCSS plugins. The indexer calls this after `extract()` and
    /// splices the resulting symbols/refs back into the same `.svelte` file.
    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        crate::languages::common::extract_html_script_style_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &["script_element", "element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element", "attribute"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        crate::languages::typescript::keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(SvelteResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![]
    }
}
