// =============================================================================
// languages/rust_lang/hooks.rs — RustHooks impl of LanguageEngineHooks.
//
// First migration: `classify_external` ports the body of
// `RustResolver::infer_external_namespace_with_lookup` (delegating to
// the existing `resolve::infer_external_inner` free function) onto the
// engine hooks seam.
// =============================================================================

use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct RustHooks;

impl LanguageEngineHooks for RustHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        resolve::infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }
}

pub static RUST_HOOKS: RustHooks = RustHooks;
