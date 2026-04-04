//! Nix language plugin.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct NixPlugin;

impl LanguagePlugin for NixPlugin {
    fn id(&self) -> &str {
        "nix"
    }

    fn language_ids(&self) -> &[&str] {
        &["nix"]
    }

    fn extensions(&self) -> &[&str] {
        &[".nix"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_nix::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let _ = source;
        ExtractionResult::empty()
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "binding",
            "inherit",
            "inherit_from",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "apply_expression",
            "with_expression",
            "select_expression",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
