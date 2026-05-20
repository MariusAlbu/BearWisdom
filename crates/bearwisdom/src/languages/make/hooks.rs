use super::resolve::is_make_builtin;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct MakeHooks;

impl LanguageEngineHooks for MakeHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if is_make_builtin(&ref_ctx.extracted_ref.target_name) {
            return Some("make".to_string());
        }
        None
    }
}

pub static MAKE_HOOKS: MakeHooks = MakeHooks;
