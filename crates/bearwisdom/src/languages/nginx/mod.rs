//! Nginx configuration plugin.
//!
//! Recognizes:
//!   * `content_by_lua_block { … }` and other `*_by_lua_block`
//!     directives → Lua regions.
//!   * `location`, `server`, `upstream` directive blocks → Field symbols.
//!
//! Tree-sitter-nginx exists upstream but isn't in the workspace — a
//! hand-rolled line scanner is sufficient for MVP.

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct NginxPlugin;

impl LanguagePlugin for NginxPlugin {
    fn id(&self) -> &str { "nginx" }
    fn language_ids(&self) -> &[&str] { &["nginx"] }
    fn extensions(&self) -> &[&str] { &[".nginx"] }
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
