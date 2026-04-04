//! dart language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod symbols;
pub mod extract;

mod builtins;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::DartResolver;

pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &str { "dart" }

    fn language_ids(&self) -> &[&str] { &["dart"] }

    fn extensions(&self) -> &[&str] { &[".dart"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_dart::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "mixin_declaration",
            "enum_declaration",
            "enum_constant",
            "extension_declaration",
            "extension_type_declaration",
            "function_signature",
            "constructor_signature",
            "factory_constructor_signature",
            "getter_signature",
            "setter_signature",
            "initialized_variable_definition",
            "type_alias",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "new_expression",
            "const_object_expression",
            "constructor_invocation",
            "library_import",
            "library_export",
            "type_arguments",
            "type_cast_expression",
            "type_test_expression",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "int", "double", "String", "bool", "void", "dynamic", "num",
            "Object", "Null", "Never", "Function", "Type",
            "Comparable", "Iterable", "Iterator", "MapEntry", "RegExp",
            "StackTrace", "StringBuffer", "StringSink", "Uri", "Zone",
            "Completer", "FutureOr", "Timer", "Isolate",
            "TypedData", "ByteBuffer", "ByteData",
            "Uint8List", "Int32List", "Float64List",
        ]
    }
}