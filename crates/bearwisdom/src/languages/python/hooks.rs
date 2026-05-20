// =============================================================================
// languages/python/hooks.rs — PythonHooks impl of LanguageEngineHooks.
//
// First migration: `classify_external` ports the body of
// `PythonResolver::infer_external_namespace_with_lookup` (which itself
// delegates to `externals::infer_external_inner`) onto the engine
// hooks seam. Legacy methods come off the resolver in the same commit.
// =============================================================================

use super::externals;
use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct PythonHooks;

impl LanguageEngineHooks for PythonHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        externals::infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        resolve::detect_flow_inner_with_lookup(file_ctx, ref_ctx, lookup)
    }
}

pub static PYTHON_HOOKS: PythonHooks = PythonHooks;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_instance_send_sync() {
        fn require<T: Send + Sync + ?Sized>() {}
        require::<PythonHooks>();
    }
}
