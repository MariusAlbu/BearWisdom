//! Pascal / Delphi language plugin.
//!
//! Grammar: tree-sitter-pascal 0.10.2 — real grammar, LANGUAGE constant available.

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod resolve;

pub use resolve::PascalResolver;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PascalPlugin;

impl LanguagePlugin for PascalPlugin {
    fn id(&self) -> &str {
        "pascal"
    }

    fn language_ids(&self) -> &[&str] {
        &["pascal", "delphi"]
    }

    fn extensions(&self) -> &[&str] {
        &[".pas", ".pp", ".dpr", ".dpk"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_pascal::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "declProc",
            "defProc",
            "declClass",
            "declIntf",
            "declSection",
            "unit",
            "declUses",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "exprCall",
            "declUses",
            "typeref",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "Integer",
            "Cardinal",
            "Int64",
            "UInt64",
            "LongInt",
            "ShortInt",
            "SmallInt",
            "Byte",
            "Word",
            "LongWord",
            "Single",
            "Double",
            "Extended",
            "Real",
            "Comp",
            "Currency",
            "Boolean",
            "ByteBool",
            "WordBool",
            "LongBool",
            "Char",
            "WideChar",
            "AnsiChar",
            "String",
            "AnsiString",
            "WideString",
            "UnicodeString",
            "ShortString",
            "PChar",
            "Pointer",
            "TObject",
            "TClass",
            "IInterface",
            "IUnknown",
            "Variant",
            "OleVariant",
        ]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PascalResolver))
    }
}
