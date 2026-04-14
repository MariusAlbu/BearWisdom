//! Haml (`.haml`) Ruby template plugin. Structurally mirrors Slim —
//! `- code`, `= expr`, filter blocks (`:javascript`, `:css`, `:scss`).

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct HamlPlugin;

impl LanguagePlugin for HamlPlugin {
    fn id(&self) -> &str { "haml" }
    fn language_ids(&self) -> &[&str] { &["haml"] }
    fn extensions(&self) -> &[&str] { &[".haml"] }
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
