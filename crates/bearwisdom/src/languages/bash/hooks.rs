// Bash hooks — classify_external delegates to the engine's common
// classifier with the bash builtin predicate.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct BashHooks;

impl LanguageEngineHooks for BashHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_bash_builtin)
    }
}

pub static BASH_HOOKS: BashHooks = BashHooks;
