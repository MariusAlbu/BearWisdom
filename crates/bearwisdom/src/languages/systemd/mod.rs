//! systemd unit files (`.service`, `.timer`, `.socket`, etc.).
//!
//!   * `Exec*=<cmd>` directives → bash regions.
//!   * `[Section]` headers and `Key=Value` pairs → Field symbols.

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct SystemdPlugin;

impl LanguagePlugin for SystemdPlugin {
    fn id(&self) -> &str { "systemd" }
    fn language_ids(&self) -> &[&str] { &["systemd"] }
    fn extensions(&self) -> &[&str] {
        &[".service", ".timer", ".socket", ".path", ".target", ".mount", ".automount"]
    }
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
