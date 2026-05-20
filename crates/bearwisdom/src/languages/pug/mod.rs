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
pub(crate) mod hooks;
pub(crate) mod profile;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

pub use hooks::PUG_HOOKS;
pub use profile::PUG_PROFILE;

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
    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PUG_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::PUG_HOOKS)
    }
}
