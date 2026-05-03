//! Pug / Jade language plugin.
//!
//! Recognizes the most common Pug constructs:
//!   * `- code`            → JavaScript statement region
//!   * `= expr`, `!= expr` → JavaScript expression region
//!   * `#{expr}`           → JavaScript expression interpolation
//!   * `include file`      → partial-include Imports ref
//!   * `extends layout`    → Imports ref
//!   * `script.` / `style.` indented blocks → JS / CSS regions
//!   * `mixin name(args)`  → Field symbol

pub mod extract;
pub mod embedded;
pub mod resolve;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct PugPlugin;

impl LanguagePlugin for PugPlugin {
    fn id(&self) -> &str { "pug" }
    fn language_ids(&self) -> &[&str] { &["pug", "jade"] }
    fn extensions(&self) -> &[&str] { &[".pug", ".jade"] }
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
        Some(std::sync::Arc::new(resolve::PugResolver))
    }
}
