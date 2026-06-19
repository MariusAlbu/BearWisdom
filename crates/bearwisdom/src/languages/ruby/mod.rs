//! ruby language plugin.

mod calls;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod params;
mod symbols;

pub mod connectors;
mod predicates;
pub(crate) mod profile;
pub use profile::RUBY_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct RubyPlugin;

impl LanguagePlugin for RubyPlugin {
    fn id(&self) -> &str {
        "ruby"
    }

    fn language_ids(&self) -> &[&str] {
        &["ruby"]
    }

    fn extensions(&self) -> &[&str] {
        &[".rb", ".rake", ".gemspec"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_ruby::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::RUBY_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class",
            "module",
            "method",
            "singleton_method",
            "singleton_class",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["call", "scope_resolution", "constant"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::RUBY_PROFILE)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::RUBY_FLOW_CONFIG)
    }
}
