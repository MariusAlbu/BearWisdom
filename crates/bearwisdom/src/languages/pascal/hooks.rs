use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct PascalHooks;

impl LanguageEngineHooks for PascalHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Pascal identifiers are case-insensitive; fold both sides so that
        // `SIZEOF`, `fillchar`, etc. classify identically to `SizeOf`.
        let target_lower = ref_ctx.extracted_ref.target_name.to_lowercase();
        let keywords = super::keywords::KEYWORDS;
        if keywords.iter().any(|k| k.to_lowercase() == target_lower) {
            return Some("primitive".to_string());
        }
        None
    }
}

pub static PASCAL_HOOKS: PascalHooks = PascalHooks;
