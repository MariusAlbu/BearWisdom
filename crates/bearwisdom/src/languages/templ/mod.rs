//! Templ (`.templ`) plugin — HTML component DSL compiled to Go.
//!
//! Minimal MVP:
//!   * `templ Name(args) { ... }` declarations → Function symbol
//!   * `@ChildComponent(args)` inside templ bodies → Calls ref
//!
//! Full Go-in-HTML parsing is deferred — for now the `{ expr }`
//! interpolation dispatches to Go as a short region.

pub mod extract;
pub mod hooks;
pub(crate) mod predicates;
pub(crate) mod profile;

pub use hooks::TEMPL_HOOKS;
pub use profile::TEMPL_PROFILE;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct TemplPlugin;

impl LanguagePlugin for TemplPlugin {
    fn id(&self) -> &str { "templ" }
    fn language_ids(&self) -> &[&str] { &["templ"] }
    fn extensions(&self) -> &[&str] { &[".templ"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::TEMPL_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::TEMPL_HOOKS)
    }
}
