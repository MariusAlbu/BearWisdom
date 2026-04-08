//! PowerShell language plugin.
//!
//! Covers `.ps1`, `.psm1`, `.psd1` files.
//!
//! What we extract:
//! - `function_statement` → Function
//! - `class_statement` → Class
//! - `enum_statement` → Enum
//! - `class_method_definition` → Method (qualified ClassName.MethodName)
//! - `class_property_definition` → Property
//! - `using_statement` / `Import-Module` commands → Imports

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod externals;
pub(crate) mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PowerShellPlugin;

impl LanguagePlugin for PowerShellPlugin {
    fn id(&self) -> &str { "powershell" }

    fn language_ids(&self) -> &[&str] { &["powershell"] }

    fn extensions(&self) -> &[&str] { &[".ps1", ".psm1", ".psd1"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_powershell::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_statement",
            "class_statement",
            "enum_statement",
            "class_method_definition",
            "class_property_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "command",
            "invokation_expression",
            "using_statement",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "string", "int", "long", "double", "float", "bool", "char", "byte",
            "object", "void", "hashtable", "array", "psobject", "pscustomobject",
            "switch", "datetime", "timespan", "guid", "uri", "regex",
        ]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn framework_globals(&self, dependencies: &std::collections::HashSet<String>) -> Vec<&'static str> {
        externals::framework_globals(dependencies)
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PowerShellResolver))
    }
}
