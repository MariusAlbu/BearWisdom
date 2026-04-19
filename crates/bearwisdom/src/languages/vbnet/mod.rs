//! VB.NET language plugin.
//!
//! Grammar: tree-sitter-vb-dotnet 0.1
//! What we extract:
//! - `class_block` → Class
//! - `module_block` → Class (VB Module = sealed static class)
//! - `structure_block` → Struct
//! - `interface_block` → Interface
//! - `enum_block` → Enum
//! - `method_declaration` → Method (Sub or Function)
//! - `property_declaration` → Property
//! - `namespace_block` → Namespace
//! - `imports_statement` → Imports edge
//! - `inherits_clause` → Inherits edge

pub(crate) mod connectors;
pub(crate) mod keywords;
pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct VbNetPlugin;

impl LanguagePlugin for VbNetPlugin {
    fn id(&self) -> &str { "vbnet" }

    fn language_ids(&self) -> &[&str] { &["vbnet"] }

    fn extensions(&self) -> &[&str] { &[".vb"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_vb_dotnet::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_block",
            "module_block",
            "structure_block",
            "interface_block",
            "enum_block",
            "enum_member",
            "constructor_declaration",
            "field_declaration",
            "const_declaration",
            "delegate_declaration",
            "event_declaration",
            "method_declaration",
            "property_declaration",
            "namespace_block",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "imports_statement",
            "invocation",
            "new_expression",
            "inherits_clause",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(crate::languages::csharp::resolve::CSharpResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![]
    }

    fn resolve_connection_points(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) -> Vec<crate::connectors::types::ConnectionPoint> {
        crate::languages::drive_connector(
            &connectors::VbNetDiConnector, db, project_root, ctx,
        )
    }

}
