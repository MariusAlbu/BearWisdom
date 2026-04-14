//! Crontab plugin — scheduled shell commands.
//!
//! Each non-comment line with a cron schedule becomes a bash region.
//! Lines of the form `KEY=value` (PATH, SHELL, etc.) are config
//! pairs emitted as Field symbols.

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct CrontabPlugin;

impl LanguagePlugin for CrontabPlugin {
    fn id(&self) -> &str { "crontab" }
    fn language_ids(&self) -> &[&str] { &["crontab"] }
    fn extensions(&self) -> &[&str] { &[".cron", ".crontab"] }
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
