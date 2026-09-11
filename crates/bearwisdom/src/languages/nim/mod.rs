//! Nim language plugin.
//!
//! Grammar status: tree-sitter-nim is not in Cargo.toml.
//! `grammar()` returns `None`; extraction is performed by a line-oriented
//! parser that recognises Nim's top-level declaration patterns.
//!
//! What we extract:
//! - `proc name(...)` → Function
//! - `func name(...)` → Function (pure)
//! - `method name(...)` → Method
//! - `template name(...)` → Function
//! - `macro name(...)` → Function
//! - `iterator name(...)` → Function
//! - `converter name(...)` → Function
//! - `type` section → scope; `TypeName = object/enum/concept/...` → Class/Struct/Enum/Interface/TypeAlias
//! - `import`, `from ... import` → Imports edges

mod external_virtual_path;
pub mod extract;
pub mod keywords;
pub(crate) mod module_paths;
mod predicates;
pub(crate) mod profile;
pub use profile::NIM_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct NimPlugin;

impl LanguagePlugin for NimPlugin {
    fn id(&self) -> &str {
        "nim"
    }

    fn language_ids(&self) -> &[&str] {
        &["nim"]
    }

    fn extensions(&self) -> &[&str] {
        &[".nim", ".nims"]
    }

    /// Returns `None` until tree-sitter-nim is added to Cargo.toml.
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn external_virtual_path(
        &self,
        _language: &str,
        normalized_absolute_path: &str,
    ) -> Option<String> {
        external_virtual_path::for_pulled(normalized_absolute_path)
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        let _ = file_path;
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "proc_declaration",
            "func_declaration",
            "method_declaration",
            "template_declaration",
            "macro_declaration",
            "iterator_declaration",
            "converter_declaration",
            "type_symbol_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
            "dot_generic_call",
            "import_statement",
            "import_from_statement",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn signature_type_application(&self, text: &str) -> (String, Vec<String>) {
        crate::languages::bracket_type_application(text)
    }

    fn signature_type_head<'a>(&self, text: &'a str) -> &'a str {
        crate::languages::bracket_type_head(text)
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::NIM_PROFILE)
    }
}
