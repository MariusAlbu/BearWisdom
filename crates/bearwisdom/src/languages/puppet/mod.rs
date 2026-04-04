//! Puppet language plugin.
//!
//! Grammar status: tree-sitter-puppet is not in Cargo.toml yet.
//! The `grammar()` method returns `None`, falling back to the generic extractor,
//! until the crate is added. The extraction logic in `extract.rs` is ready for
//! when the grammar is wired in.

pub mod extract;

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
    /// Falls back to the generic extractor in the meantime.
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        // Grammar not available yet — return empty until grammar is wired in.
        // extract::extract(source) is ready for when it is.
        let _ = source;
        ExtractionResult::empty()
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
}
