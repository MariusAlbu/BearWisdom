use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct PuppetHooks;

impl LanguageEngineHooks for PuppetHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let _ = lookup;
        resolve::infer_external_inner(file_ctx, ref_ctx, project_ctx)
    }
}

pub static PUPPET_HOOKS: PuppetHooks = PuppetHooks;