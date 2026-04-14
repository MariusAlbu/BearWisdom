//! Nunjucks (`.njk`) language plugin.
//!
//! Jinja2-compatible syntax:
//!   * `{{ expr }}`                  → JavaScript expression region
//!   * `{% tag … %}`                 → directive (may be a symbol)
//!   * `{% extends "base.njk" %}`    → Imports ref
//!   * `{% include "partial.njk" %}` → Imports ref
//!   * `{# comment #}`               → skip
//!
//! Tree-sitter-nunjucks exists upstream but isn't in the workspace;
//! a hand-rolled scanner is sufficient for MVP.

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct NunjucksPlugin;

impl LanguagePlugin for NunjucksPlugin {
    fn id(&self) -> &str { "nunjucks" }
    fn language_ids(&self) -> &[&str] { &["nunjucks", "njk"] }
    fn extensions(&self) -> &[&str] { &[".njk", ".nunjucks"] }
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
