//! Plain HTML (`.html`, `.htm`, `.xhtml`) language plugin.
//!
//! The host file produces a file-stem symbol plus one anchor symbol per
//! element with an `id="…"` attribute (so `file_symbols` on an HTML
//! page lists jumpable anchors). Embedded regions are the standard
//! HTML-host shape — `<script>` blocks become JS / TS regions,
//! `<style>` blocks become CSS / SCSS regions. JSON-typed scripts
//! (`type="application/json"`, `type="application/ld+json"`) are
//! skipped so structured-data blobs don't produce parser noise.
//!
//! The heavy lifting (tree walk, attribute read, origin mapping) is
//! shared with Vue/Svelte/Astro via `languages::common::extract_html_script_style_regions`.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct HtmlPlugin;

impl LanguagePlugin for HtmlPlugin {
    fn id(&self) -> &str {
        "html"
    }

    fn language_ids(&self) -> &[&str] {
        &["html"]
    }

    fn extensions(&self) -> &[&str] {
        &[".html", ".htm", ".xhtml"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_html::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        // Generated HTML docs (Robot Framework reports, JavaDoc, pydoc)
        // bundle full minified jQuery into <script> tags. Skipping host
        // extraction without skipping embedded regions would still let
        // the JS extractor walk those bundles and emit ~150 K symbols
        // per docs file. The `looks_generated_html` predicate is shared
        // with `extract::extract` for symmetric behaviour.
        if extract::looks_generated_html(source) {
            return Vec::new();
        }
        crate::languages::common::extract_html_script_style_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element"]
    }
}
