use super::resolve;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct CobolHooks;

impl LanguageEngineHooks for CobolHooks {
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

pub static COBOL_HOOKS: CobolHooks = CobolHooks;