//! Handlebars / Mustache language plugin.
//!
//! Recognizes:
//!   * `{{ expr }}` / `{{{ raw }}}`             — JavaScript expression region
//!   * `{{#each xs}}...{{/each}}`               — block symbol
//!   * `{{#if cond}}...{{/if}}`                 — block symbol
//!   * `{{> partial}}`                          — partial-include Imports edge
//!   * `<script>` / `<style>` in HTML sections  — JS / CSS regions

pub mod extract;
pub mod embedded;
pub(crate) mod hooks;
pub(crate) mod profile;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

pub use hooks::HANDLEBARS_HOOKS;
pub use profile::HANDLEBARS_PROFILE;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct HandlebarsPlugin;

impl LanguagePlugin for HandlebarsPlugin {
    fn id(&self) -> &str { "handlebars" }
    fn language_ids(&self) -> &[&str] { &["handlebars", "hbs", "mustache"] }
    fn extensions(&self) -> &[&str] { &[".hbs", ".handlebars", ".mustache"] }
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
        Some(&profile::HANDLEBARS_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::HANDLEBARS_HOOKS)
    }
}
