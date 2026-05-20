// =============================================================================
// languages/swift/hooks.rs — SwiftHooks impl of LanguageEngineHooks.
// `classify_external` delegates to `resolve::infer_external_inner`.
// =============================================================================

use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct SwiftHooks;

impl LanguageEngineHooks for SwiftHooks {
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

pub static SWIFT_HOOKS: SwiftHooks = SwiftHooks;
