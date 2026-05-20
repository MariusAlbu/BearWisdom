// SQL hooks — classify_external delegates to engine's common classifier
// with the SQL builtin-type predicate.

use super::resolve;
use super::resolve::is_sql_builtin_type;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct SqlHooks;

impl LanguageEngineHooks for SqlHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_sql_builtin_type)
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }
}

pub static SQL_HOOKS: SqlHooks = SqlHooks;
