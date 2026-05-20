// =============================================================================
// languages/typescript/hooks.rs — TypeScriptHooks impl of LanguageEngineHooks.
// =============================================================================

use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct TypeScriptHooks;

impl LanguageEngineHooks for TypeScriptHooks {
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

/// Static instance the language plugin returns. `'static` so the engine
/// can store `&'static dyn LanguageEngineHooks` in its hook registry
/// without lifetime gymnastics.
pub static TYPESCRIPT_HOOKS: TypeScriptHooks = TypeScriptHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
