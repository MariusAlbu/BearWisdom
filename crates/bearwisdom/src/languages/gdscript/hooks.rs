use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct GdScriptHooks;

impl LanguageEngineHooks for GdScriptHooks {
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
        super::resolve::GDScriptResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static GDSCRIPT_HOOKS: GdScriptHooks = GdScriptHooks;