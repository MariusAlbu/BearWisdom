//! EEx (`.eex`, `.leex`, `.html.eex`) — legacy Elixir templates.
//! Reuses HEEx's embedded logic for `<% %>`/`<%= %>` and falls back
//! to plain file-stem symbol.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct EexPlugin;

impl LanguagePlugin for EexPlugin {
    fn id(&self) -> &str { "eex" }
    fn language_ids(&self) -> &[&str] { &["eex", "leex"] }
    fn extensions(&self) -> &[&str] { &[".eex", ".leex", ".html.eex"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn embedded_regions(&self, s: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        // Reuse HEEx's EEx-style `<% %>` scanner — same semantics.
        crate::languages::heex::embedded::detect_regions(s)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
