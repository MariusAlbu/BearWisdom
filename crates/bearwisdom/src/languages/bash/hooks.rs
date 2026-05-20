// Bash hooks — classify_external delegates to the engine's common
// classifier with the bash builtin predicate.

use super::resolve;
use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup, Resolution};
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
        use crate::indexer::resolve::engine::LanguageResolver;
        super::resolve::BashResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static BASH_HOOKS: BashHooks = BashHooks;
