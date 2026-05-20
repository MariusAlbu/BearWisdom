use super::resolve;
use super::resolve::is_graphql_builtin;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup, Resolution};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct GraphQlHooks;

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
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &crate::indexer::resolve::engine::FileContext,
        ref_ctx: &crate::indexer::resolve::engine::RefContext<'_>,
        lookup: &dyn crate::indexer::resolve::engine::SymbolLookup,
    ) -> Option<crate::indexer::resolve::engine::Resolution> {
        super::resolve::GraphQlResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static GRAPHQL_HOOKS: GraphQlHooks = GraphQlHooks;
