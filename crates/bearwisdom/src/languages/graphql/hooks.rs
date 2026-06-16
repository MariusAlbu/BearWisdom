// GraphQL language hooks. Absorbed from the deleted `graphql/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct GraphQlHooks;

/// GraphQL built-in scalar types and introspection system types.
pub(crate) fn is_graphql_builtin(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Int"
            | "Float"
            | "Boolean"
            | "ID"
            | "__Schema"
            | "__Type"
            | "__Field"
            | "__InputValue"
            | "__EnumValue"
            | "__Directive"
            | "__DirectiveLocation"
            | "__TypeKind"
    )
}

impl LanguageEngineHooks for GraphQlHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_graphql_builtin)
            .map(|_| "graphql".to_string())
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "graphql".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        })
    }
}

pub static GRAPHQL_HOOKS: GraphQlHooks = GraphQlHooks;
