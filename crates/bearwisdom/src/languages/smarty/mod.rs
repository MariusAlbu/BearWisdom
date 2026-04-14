//! Smarty PHP templates.
//!
//!   * `{$var}`, `{func($x)}` → Smarty expression region (dispatched
//!     as `php` for a rough approximation — Smarty doesn't have its
//!     own extractor today).
//!   * `{include file="p.tpl"}` / `{extends file="base.tpl"}` → Imports ref

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct SmartyPlugin;

impl LanguagePlugin for SmartyPlugin {
    fn id(&self) -> &str { "smarty" }
    fn language_ids(&self) -> &[&str] { &["smarty"] }
    fn extensions(&self) -> &[&str] { &[".smarty", ".smarty.tpl"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
