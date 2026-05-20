use super::resolve::is_graphql_builtin;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup};
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
}

pub static GRAPHQL_HOOKS: GraphQlHooks = GraphQlHooks;
