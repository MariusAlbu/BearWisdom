use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct ElixirHooks;

impl LanguageEngineHooks for ElixirHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        resolve::infer_external_inner_with_lookup(file_ctx, ref_ctx, project_ctx, lookup)
    }
}

pub static ELIXIR_HOOKS: ElixirHooks = ElixirHooks;
