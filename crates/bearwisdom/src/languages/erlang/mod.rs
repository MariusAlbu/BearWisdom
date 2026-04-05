//! Erlang language plugin.
//!
//! Covers `.erl` and `.hrl` files.
//!
//! What we extract:
//! - `fun_decl` → Function (name/arity)
//! - `module_attribute` → Namespace
//! - `record_decl` → Struct
//! - `behaviour_attribute` → Implements edge
//! - `export_attribute` → marks functions as public
//! - `import_attribute` / `pp_include` → Imports

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ErlangPlugin;

impl LanguagePlugin for ErlangPlugin {
    fn id(&self) -> &str { "erlang" }

    fn language_ids(&self) -> &[&str] { &["erlang"] }

    fn extensions(&self) -> &[&str] { &[".erl", ".hrl"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_erlang::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "fun_decl",
            "module_attribute",
            "record_decl",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
            "import_attribute",
            "pp_include",
            "pp_include_lib",
            "behaviour_attribute",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "atom", "integer", "float", "boolean", "binary", "bitstring",
            "list", "tuple", "map", "pid", "port", "reference", "fun",
            "iolist", "iodata", "string", "char", "byte", "timeout",
            "any", "none", "term", "number",
        ]
    }
}
