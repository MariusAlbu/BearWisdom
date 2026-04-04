//! GraphQL language plugin.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GraphQlPlugin;

impl LanguagePlugin for GraphQlPlugin {
    fn id(&self) -> &str {
        "graphql"
    }

    fn language_ids(&self) -> &[&str] {
        &["graphql"]
    }

    fn extensions(&self) -> &[&str] {
        &[".graphql", ".gql"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_graphql::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let _ = source;
        ExtractionResult::empty()
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
