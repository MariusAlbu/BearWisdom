//! Dockerfile language plugin.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct DockerfilePlugin;

impl LanguagePlugin for DockerfilePlugin {
    fn id(&self) -> &str {
        "dockerfile"
    }

    fn language_ids(&self) -> &[&str] {
        &["dockerfile"]
    }

    /// No file extensions — Dockerfile detection is handled by bearwisdom-profile
    /// via filename matching ("Dockerfile", "*.dockerfile", etc.).
    fn extensions(&self) -> &[&str] {
        &[]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_dockerfile_0_25::LANGUAGE.into())
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
            "from_instruction",
            "arg_instruction",
            "env_instruction",
            "label_instruction",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "copy_instruction",
            "from_instruction",
        ]
    }
}
