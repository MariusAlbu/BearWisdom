use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct SvelteHooks;

impl LanguageEngineHooks for SvelteHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        crate::languages::typescript::hooks::infer_external_inner_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        )
    }
}

pub static SVELTE_HOOKS: SvelteHooks = SvelteHooks;
