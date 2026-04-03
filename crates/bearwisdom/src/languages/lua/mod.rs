//! Lua language plugin.
//!
//! Grammar: tree-sitter-lua (in Cargo.toml).
//! Extraction covers top-level functions, table-based OOP, require imports, and calls.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

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
            "function_definition",
            "variable_declaration",
            "assignment_statement",
            "field",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call",
            "method_index_expression",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "string", "number", "boolean", "nil", "table", "function",
            "thread", "userdata", "integer", "float",
        ]
    }
}
