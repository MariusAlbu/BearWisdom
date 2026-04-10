//! Puppet language plugin.

pub mod primitives;
pub mod extract;
pub mod resolve;
mod builtins;
pub(crate) mod externals;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PuppetPlugin;

impl LanguagePlugin for PuppetPlugin {
    fn id(&self) -> &str {
        "puppet"
    }

    fn language_ids(&self) -> &[&str] {
        &["puppet"]
    }

    fn extensions(&self) -> &[&str] {
        &[".pp"]
    }

    /// Returns `None` until tree-sitter-puppet is added to Cargo.toml.
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_puppet::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source, tree_sitter_puppet::LANGUAGE.into())
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "defined_resource_type",
            "function_declaration",
            "node_definition",
            "resource_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "include_statement",
            "require_statement",
            "function_call",
            "resource_declaration",
            "resource_reference",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            // Puppet core types
            "String",
            "Integer",
            "Float",
            "Boolean",
            "Array",
            "Hash",
            "Undef",
            "Optional",
            "Variant",
            "Enum",
            "Pattern",
            "Regexp",
            "Callable",
            "Type",
            "Any",
            "Scalar",
            "Collection",
            "Numeric",
            "Data",
            "Resource",
            "Class",
            "Tuple",
            "Struct",
            "Iterable",
            "Iterator",
            "NotUndef",
            "Sensitive",
        ]
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn framework_globals(&self, dependencies: &std::collections::HashSet<String>) -> Vec<&'static str> {
        externals::framework_globals(dependencies)
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PuppetResolver))
    }
}
