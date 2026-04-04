//! Perl language plugin.
//!
//! Covers `.pl` and `.pm` files.
//!
//! Note: tree-sitter-perl 1.1 has a hard dependency on tree-sitter 0.26 which
//! conflicts with the workspace's tree-sitter 0.25.  This plugin therefore uses
//! a line-oriented scanner rather than a tree-sitter grammar.
//!
//! What we extract:
//! - `sub name` → Function
//! - `package Name::Space` → Namespace
//! - `use Module` → Imports

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PerlPlugin;

impl LanguagePlugin for PerlPlugin {
    fn id(&self) -> &str { "perl" }

    fn language_ids(&self) -> &[&str] { &["perl"] }

    fn extensions(&self) -> &[&str] { &[".pl", ".pm"] }

    /// No grammar available: tree-sitter-perl 1.1 requires tree-sitter 0.26
    /// (ABI conflict). Returns None so the plugin falls through to line scanning.
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> { None }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Line-based scanner — no tree-sitter node kinds
        &[]
    }

    fn ref_node_kinds(&self) -> &[&str] { &[] }

    fn builtin_type_names(&self) -> &[&str] { &[] }
}
