//! Jinja2 (`.jinja`, `.jinja2`, `.j2`) language plugin.
//!
//! Native extractor — does NOT delegate to the Nunjucks plugin or embed
//! `{{ ... }}` payloads as JavaScript. The legacy "Jinja-as-JS" embedding
//! produced phantom Calls refs for every Jinja filter (`indent`,
//! `to_nice_yaml`, `regex_replace`, `lookup`, ...) because pipe-filter
//! syntax doesn't survive a JS literal embed and the JS extractor sees the
//! filter names as function calls.
//!
//! Foundation phase (PR-1):
//!   * Native scanner emits identifier-chain TypeRefs from `{{ x.y.z }}`
//!     payloads; symbol-introducing directives (`{% block %}`,
//!     `{% extends %}`, `{% include %}`, `{% import %}`, `{% from %}`).
//!   * `embedded_regions` returns empty — no JS routing.
//!
//! Follow-up phases (separate sessions):
//!   * Filter-call refs against a synthetic Jinja2 stdlib module.
//!   * Function-call recognition (`lookup(...)`, `range(...)`).
//!   * Ansible-specific globals scanned out of the host inventory.
//!   * `{% set %}` symbol declarations + `{% for %}` scope variables.

pub mod expr;
pub mod extract;
pub(crate) mod hooks;
pub(crate) mod profile;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

pub use hooks::JINJA_HOOKS;
pub use profile::JINJA_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use std::sync::Arc;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct JinjaPlugin;

impl LanguagePlugin for JinjaPlugin {
    fn id(&self) -> &str {
        "jinja"
    }
    fn language_ids(&self) -> &[&str] {
        &["jinja", "jinja2", "j2"]
    }
    fn extensions(&self) -> &[&str] {
        &[".jinja", ".jinja2", ".j2"]
    }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> {
        None
    }
    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn embedded_regions(&self, _s: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        // No JS routing — see module docs for rationale.
        Vec::new()
    }
    fn symbol_node_kinds(&self) -> &[&str] {
        &[]
    }
    fn ref_node_kinds(&self) -> &[&str] {
        &[]
    }
    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::JINJA_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&hooks::JINJA_HOOKS)
    }
}
