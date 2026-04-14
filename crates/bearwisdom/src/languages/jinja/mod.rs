//! Jinja2 (`.jinja`, `.j2`) plugin.
//!
//! Syntax is nearly identical to Nunjucks — the Nunjucks plugin's
//! extract + embedded modules already handle `{{ expr }}`, `{% tag %}`,
//! `{% extends %}`, `{% include %}`. We wrap that logic here with a
//! distinct plugin id so `.j2` files are detected as Jinja.

pub struct JinjaPlugin;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

impl LanguagePlugin for JinjaPlugin {
    fn id(&self) -> &str { "jinja" }
    fn language_ids(&self) -> &[&str] { &["jinja", "jinja2", "j2"] }
    fn extensions(&self) -> &[&str] { &[".jinja", ".jinja2", ".j2"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        // Reuse Nunjucks' extractor — same directive vocabulary.
        crate::languages::nunjucks::extract::extract(s, p)
    }
    fn embedded_regions(&self, s: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        crate::languages::nunjucks::embedded::detect_regions(s)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
