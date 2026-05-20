use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct ScalaHooks;

impl LanguageEngineHooks for ScalaHooks {
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

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        resolve::detect_flow_inner(file_ctx, ref_ctx)
    }
}

pub static SCALA_HOOKS: ScalaHooks = ScalaHooks;
