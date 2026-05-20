use super::resolve::infer_r_external;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct RHooks;

impl LanguageEngineHooks for RHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some(target.clone());
        }
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if file_ctx
                .imports
                .iter()
                .any(|i| i.module_path.as_deref() == Some(module.as_str()))
            {
                return Some(module.clone());
            }
        }
        infer_r_external(file_ctx, ref_ctx, project_ctx)
    }
}

pub static R_HOOKS: RHooks = RHooks;
