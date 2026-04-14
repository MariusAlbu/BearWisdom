//! Mako (`.mako`, `.html.mako`) Python template plugin.
//!
//!   * `${expr}`                  → Python expression region
//!   * `<% code %>`               → Python statement
//!   * `<%def name="foo()"> ... </%def>` → Field symbol
//!   * `<%include file="x"/>`     → Imports ref
//!   * `<%inherit file="base"/>`  → Imports ref

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct MakoPlugin;

impl LanguagePlugin for MakoPlugin {
    fn id(&self) -> &str { "mako" }
    fn language_ids(&self) -> &[&str] { &["mako"] }
    fn extensions(&self) -> &[&str] { &[".mako", ".html.mako"] }
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
