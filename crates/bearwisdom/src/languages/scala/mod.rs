//! scala language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
pub(crate) mod primitives;
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

pub use resolve::ScalaResolver;

pub struct ScalaPlugin;

impl LanguagePlugin for ScalaPlugin {
    fn id(&self) -> &str { "scala" }

    fn language_ids(&self) -> &[&str] { &["scala"] }

    fn extensions(&self) -> &[&str] { &[".scala", ".sc"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_scala::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::SCALA_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "object_definition",
            "trait_definition",
            "enum_definition",
            "full_enum_case",
            "simple_enum_case",
            "function_definition",
            "function_declaration",
            "val_definition",
            "var_definition",
            "val_declaration",
            "var_declaration",
            "type_definition",
            "given_definition",
            "package_clause",
            "package_object",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "instance_expression",
            "import_declaration",
            "export_declaration",
            "type_identifier",
            // type_arguments is intentionally excluded: generic type params like
            // `class Foo[A <: Bar, B]` produce multiple refs per node, breaking
            // the 1:1 node→ref coverage assumption (budget system only credits 1).
            // extends_clause is similarly excluded: `class Foo extends Bar with Baz`
            // produces both Inherits(Bar) and Implements(Baz) from one CST node.
            "infix_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::ScalaResolver))
    }

}