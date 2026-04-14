//! Slim (`.slim`) Ruby template plugin.
//!
//! Line-based:
//!   * `= expr`, `== expr`        → Ruby expression
//!   * `- code`                   → Ruby statement
//!   * `ruby:`                    → Ruby block
//!   * `javascript:`              → JS block
//!   * `css:`                     → CSS block

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct SlimPlugin;

impl LanguagePlugin for SlimPlugin {
    fn id(&self) -> &str { "slim" }
    fn language_ids(&self) -> &[&str] { &["slim"] }
    fn extensions(&self) -> &[&str] { &[".slim"] }
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
