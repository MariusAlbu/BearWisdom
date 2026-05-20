//! Lua language plugin.
//!
//! Grammar: tree-sitter-lua (in Cargo.toml).
//! Extraction covers top-level functions, table-based OOP, require imports, and calls.

pub mod keywords;
pub mod extract;
pub mod flow;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

pub use hooks::LUA_HOOKS;
pub use profile::LUA_PROFILE;

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

    fn keywords(&self) -> &'static [&'static str] { keywords::KEYWORDS }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::LUA_PROFILE)
    }

    
    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::LUA_HOOKS)
    }
fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::LUA_FLOW_CONFIG)
    }
}
