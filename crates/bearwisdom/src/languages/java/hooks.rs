// =============================================================================
// languages/java/hooks.rs — JavaHooks impl of LanguageEngineHooks.
//
// Hosts engine-side per-language behaviors that used to live on
// `JavaResolver`. First migration: `classify_external` delegates to
// `resolve::infer_external_inner` (same helper the legacy methods
// called).
// =============================================================================

use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct JavaHooks;

impl LanguageEngineHooks for JavaHooks {
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

pub static JAVA_HOOKS: JavaHooks = JavaHooks;
