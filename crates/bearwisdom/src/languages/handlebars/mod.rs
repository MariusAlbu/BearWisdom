//! Handlebars / Mustache language plugin.
//!
//! Recognizes:
//!   * `{{ expr }}` / `{{{ raw }}}`             — JavaScript expression region
//!   * `{{#each xs}}...{{/each}}`               — block symbol
//!   * `{{#if cond}}...{{/if}}`                 — block symbol
//!   * `{{> partial}}`                          — partial-include Imports edge
//!   * `<script>` / `<style>` in HTML sections  — JS / CSS regions

pub mod extract;
pub mod embedded;
pub mod resolve;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct HandlebarsPlugin;

impl LanguagePlugin for HandlebarsPlugin {
    fn id(&self) -> &str { "handlebars" }
    fn language_ids(&self) -> &[&str] { &["handlebars", "hbs", "mustache"] }
    fn extensions(&self) -> &[&str] { &[".hbs", ".handlebars", ".mustache"] }
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
    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::HandlebarsResolver))
    }
}
