//! c_lang language plugin.

mod calls;
mod declarations;
pub mod extract;
mod flow;
mod helpers;
pub mod keywords;
pub mod macro_catalog;
mod macro_misparse;
mod preproc;
mod salvage_callconv;
mod salvage_defines;
mod salvage_funcptr;
mod salvage_macro_expand;
mod salvage_template_class;
mod salvage_text;
mod templates;
mod type_refs;
mod typerefs;
mod visitor;
mod predicates;
pub(crate) mod profile;
pub use profile::C_LANG_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "declarations_tests.rs"]
mod declarations_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "type_refs_tests.rs"]
mod type_refs_tests;

#[cfg(test)]
#[path = "macro_catalog_tests.rs"]
mod macro_catalog_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct CLangPlugin;

impl LanguagePlugin for CLangPlugin {
    fn id(&self) -> &str {
        "c_lang"
    }

    fn language_ids(&self) -> &[&str] {
        &["c", "cpp"]
    }

    fn extensions(&self) -> &[&str] {
        &[".c", ".h", ".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"]
    }

    /// Route per extension so `.c`/`.h` use the C grammar and `.cpp` etc.
    /// use the C++ grammar. Without this override the default impl returns
    /// `id() = "c_lang"`, which the registry's `by_lang_id` table doesn't
    /// know about — every `.c`/`.h`/`.cpp` file then routes to the generic
    /// fallback plugin and emits zero symbols. Mirrors `TypeScriptPlugin`
    /// which does the same `.ts`/`.tsx` split. (Same id-mismatch family
    /// as the rust_lang fix in PR 104.)
    fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
        match ext.to_ascii_lowercase().as_str() {
            ".c" | ".h" => Some("c"),
            ".cpp" | ".cc" | ".cxx" | ".hpp" | ".hh" | ".hxx" => Some("cpp"),
            _ => None,
        }
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        match lang_id {
            "c" => Some(tree_sitter_c::LANGUAGE.into()),
            "cpp" => Some(tree_sitter_cpp::LANGUAGE.into()),
            _ => None,
        }
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::C_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        extract::extract_with_file(source, file_path, lang_id)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Only register node kinds where extraction reliably covers >= 95% of
        // all occurrences.
        //
        // Excluded from tracking:
        //   struct_specifier / union_specifier / enum_specifier — these nodes
        //   appear both as DEFINITIONS (struct Foo { ... }) and as TYPE
        //   REFERENCES (struct Foo *ptr). We only emit symbols for definitions;
        //   the type-reference occurrences inflate the denominator and make 95%
        //   coverage structurally impossible to achieve.
        //   field_declaration — nested field declarations inside anonymous or
        //   complex struct hierarchies are not always reachable.
        //   type_definition — typedef bodies may contain unnamed specifiers whose
        //   inner symbol is emitted at a different line from the typedef wrapper.
        //
        // NOTE: template_declaration is intentionally excluded — the extractor
        // emits the symbol for the inner node (function_definition, class_specifier,
        // etc.) which is already tracked via those kinds.
        &[
            "function_definition",
            "declaration",
            "enumerator",
            "preproc_def",
            "preproc_function_def",
            // C++ additions
            "class_specifier",
            "namespace_definition",
            "namespace_alias_definition",
            "alias_declaration",
            "concept_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "preproc_include",
            "type_identifier",
            "base_class_clause",
            "cast_expression",
            "sizeof_expression",
            "template_argument_list",
            // C++ import (C++20 modules)
            "import_declaration",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::C_LANG_PROFILE)
    }


    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::C_FLOW_CONFIG)
    }
}
