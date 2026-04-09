//! csharp language plugin.

mod calls;
pub mod connectors;
pub(crate) mod decorators;
pub(crate) mod externals;
mod helpers;
pub(crate) mod primitives;
mod symbols;
mod types;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::CSharpResolver;

pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &str { "csharp" }

    fn language_ids(&self) -> &[&str] { &["csharp"] }

    fn extensions(&self) -> &[&str] { &[".cs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_c_sharp::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::CSHARP_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "struct_declaration",
            "record_declaration",
            "interface_declaration",
            "enum_declaration",
            "enum_member_declaration",
            "delegate_declaration",
            "event_declaration",
            "event_field_declaration",
            "method_declaration",
            "constructor_declaration",
            "destructor_declaration",
            "property_declaration",
            "indexer_declaration",
            "operator_declaration",
            "conversion_operator_declaration",
            "field_declaration",
            "local_function_statement",
            "namespace_declaration",
            "file_scoped_namespace_declaration",
            "accessor_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "invocation_expression",
            "object_creation_expression",
            "implicit_object_creation_expression",
            "using_directive",
            "base_list",
            "type_argument_list",
            "cast_expression",
            "is_expression",
            "as_expression",
            "typeof_expression",
            "attribute",
            "generic_name",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &["int", "string", "bool", "void", "float", "double", "decimal", "byte", "sbyte", "short", "ushort", "uint", "long", "ulong", "char", "object", "dynamic", "var", "nint", "nuint"]
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
        Some(std::sync::Arc::new(resolve::CSharpResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![
            Box::new(connectors::DotnetDiConnector),
            Box::new(connectors::EventBusConnector),
            Box::new(connectors::CSharpGrpcConnector),
            Box::new(connectors::CSharpMqConnector),
            Box::new(connectors::CSharpGraphQlConnector),
            Box::new(connectors::CsharpRestConnector),
        ]
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        _project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        if let Err(e) = connectors::run_ef_core(db) {
            tracing::warn!("EF Core post-index hook: {e}");
        }
    }
}
