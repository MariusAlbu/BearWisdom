//! Phoenix HEEx (`.heex`) plugin — HTML + Embedded Elixir.
//!
//! Recognizes:
//!   * `{ expr }`        → Elixir expression
//!   * `<%= expr %>`     → Elixir expression (legacy EEx)
//!   * `<% code %>`      → Elixir statement
//!   * `<.component />`  → Calls ref (component usage)
//!   * `<script>` / `<style>` blocks → JS / CSS regions

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct HeexPlugin;

impl LanguagePlugin for HeexPlugin {
    fn id(&self) -> &str { "heex" }
    fn language_ids(&self) -> &[&str] { &["heex"] }
    fn extensions(&self) -> &[&str] { &[".heex"] }
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
