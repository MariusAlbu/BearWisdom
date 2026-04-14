//! Liquid (`.liquid`) plugin — Jekyll / Shopify templates.
//!
//! Shares Nunjucks' extract+embedded (both use `{{ expr }}` and
//! `{% tag %}`). Liquid-specific tags like `section`, `layout`,
//! `render` resolve through the same include/extends Imports ref
//! path.

pub struct LiquidPlugin;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

impl LanguagePlugin for LiquidPlugin {
    fn id(&self) -> &str { "liquid" }
    fn language_ids(&self) -> &[&str] { &["liquid"] }
    fn extensions(&self) -> &[&str] { &[".liquid"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        crate::languages::nunjucks::extract::extract(s, p)
    }
    fn embedded_regions(&self, s: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        crate::languages::nunjucks::embedded::detect_regions(s)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
