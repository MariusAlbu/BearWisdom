//! Lua language plugin.
//!
//! Grammar: tree-sitter-lua (in Cargo.toml).
//! Extraction covers top-level functions, table-based OOP, require imports, and calls.

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod externals;
pub(crate) mod resolve;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

pub struct LuaPlugin;

impl LanguagePlugin for LuaPlugin {
    fn id(&self) -> &str { "lua" }

    fn language_ids(&self) -> &[&str] { &["lua"] }

    fn extensions(&self) -> &[&str] { &[".lua"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_lua::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::LUA_SCOPE_KINDS
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_declaration",
            "local_function",
            "variable_declaration",
            "assignment_statement",
            "field",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "string", "number", "boolean", "nil", "table", "function",
            "thread", "userdata", "integer", "float",
        ]
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::LuaResolver))
    }

    fn externals_locator(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::indexer::externals::ExternalSourceLocator>> {
        Some(std::sync::Arc::new(
            crate::indexer::externals::LuaExternalsLocator,
        ))
    }
}
