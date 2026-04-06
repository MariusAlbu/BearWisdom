//! ruby language plugin.

mod calls;
mod helpers;
mod params;
pub(crate) mod primitives;
mod symbols;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::RubyResolver;

pub struct RubyPlugin;

impl LanguagePlugin for RubyPlugin {
    fn id(&self) -> &str { "ruby" }

    fn language_ids(&self) -> &[&str] { &["ruby"] }

    fn extensions(&self) -> &[&str] { &[".rb", ".rake", ".gemspec"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_ruby::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::RUBY_SCOPE_KINDS }

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
        &[
            "call",
            "scope_resolution",
            "constant",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::RubyResolver))
    }
}