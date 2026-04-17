//! Astro language plugin.
//!
//! Astro Island framework files (.astro) are parsed using the HTML grammar as a
//! structural fallback. There is no tree-sitter-astro in the workspace.
//!
//! What we extract at this grammar level:
//! - The file itself → a Class symbol (component name inferred from filename)
//! - PascalCase tags in the template → Calls edges (component usages, island components)
//! - Kebab-case custom element tags → Calls edges (normalised to PascalCase)
//! - `client:*` hydration directive attributes → Calls edges to the component
//!   (already captured via PascalCase tag detection; directive noted as metadata)
//! - The frontmatter fencing (`---`) block is a JS/TS injection point for the
//!   JS extractor; we note its presence but do not parse it here.
//!
//! The frontmatter `---` block content is handled by the JS/TS extractor when
//! the indexer processes the embedded text as a separate extraction target.

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct AstroPlugin;

impl LanguagePlugin for AstroPlugin {
    fn id(&self) -> &str {
        "astro"
    }

    fn language_ids(&self) -> &[&str] {
        &["astro"]
    }

    fn extensions(&self) -> &[&str] {
        &[".astro"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // HTML grammar handles the page outer shell (template markup).
        Some(tree_sitter_html::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    /// Extract the `---`-delimited TypeScript frontmatter block plus any
    /// inline `<script>` / `<style>` blocks in the markup. The frontmatter
    /// holds the imports and data-fetching code the JS extractor needs to
    /// resolve component references against.
    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        let mut regions = crate::languages::common::extract_html_script_style_regions(source);
        if let Some(frontmatter) = crate::languages::common::extract_astro_frontmatter(source) {
            regions.insert(0, frontmatter);
        }
        regions
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element", "attribute"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        crate::languages::typescript::keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(crate::languages::typescript::resolve::TypeScriptResolver))
    }

}
