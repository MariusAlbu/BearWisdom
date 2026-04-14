//! ERB (`.erb`, `.html.erb`, `.rhtml`) language plugin.
//!
//! ERB tokens:
//!   * `<% code %>`   → Ruby statement region
//!   * `<%= expr %>`  → Ruby expression region
//!   * `<%- ... -%>`  → trim variants, same as plain
//!   * `<%# … %>`     → comment, skipped
//!
//! Inline `<script>` / `<style>` blocks dispatch via
//! `extract_html_script_style_regions`.

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct ErbPlugin;

impl LanguagePlugin for ErbPlugin {
    fn id(&self) -> &str { "erb" }
    fn language_ids(&self) -> &[&str] { &["erb"] }
    fn extensions(&self) -> &[&str] { &[".erb", ".html.erb", ".rhtml"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn embedded_regions(&self, s: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(s)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
