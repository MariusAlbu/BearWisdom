//! Groovy language plugin.
//!
//! Covers `.groovy` and `.gradle` files.
//!
//! What we extract:
//! - `class_declaration` → Class
//! - `function_definition` → Function (top-level `def`)
//! - `method_declaration` → Method (typed, inside class body)
//! - `package_declaration` → Namespace
//! - `import_declaration` → Imports
//! - `method_invocation` → Calls

pub(crate) mod connectors;
pub(crate) mod keywords;
pub mod extract;
mod ast_visit;
mod calls;
mod flow;
mod node_helpers;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;

pub use hooks::GROOVY_HOOKS;
pub use profile::GROOVY_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GroovyPlugin;

impl LanguagePlugin for GroovyPlugin {
    fn id(&self) -> &str { "groovy" }

    fn language_ids(&self) -> &[&str] { &["groovy"] }

    fn extensions(&self) -> &[&str] { &[".groovy", ".gradle"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_groovy::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "function_definition",
            "method_declaration",
            "package_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "method_invocation",
            "import_declaration",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::GROOVY_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::GROOVY_HOOKS)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::GROOVY_FLOW_CONFIG)
    }
}
