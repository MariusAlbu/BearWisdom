//! EJS (`.ejs`) language plugin.
//!
//! EJS tokens:
//!   * `<% code %>`   → JavaScript statement region
//!   * `<%= expr %>`  → JavaScript expression region (escaped)
//!   * `<%- expr %>`  → JavaScript expression region (raw)
//!   * `<%# … %>`     → comment, skipped
//!   * `<%% `/`%%>`   → literal delimiters, skipped
//!
//! HTML outside the tokens passes through
//! `extract_html_script_style_regions` to pick up `<script>`/`<style>`
//! blocks.

pub mod extract;
pub mod embedded;
pub mod resolve;

use std::sync::Arc;

use crate::indexer::resolve::engine::LanguageResolver;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct EjsPlugin;

impl LanguagePlugin for EjsPlugin {
    fn id(&self) -> &str { "ejs" }
    fn language_ids(&self) -> &[&str] { &["ejs"] }
    fn extensions(&self) -> &[&str] { &[".ejs"] }
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
    fn resolver(&self) -> Option<Arc<dyn LanguageResolver>> {
        Some(Arc::new(resolve::EjsResolver))
    }
}
