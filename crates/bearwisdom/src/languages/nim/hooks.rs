use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct NimHooks;

impl LanguageEngineHooks for NimHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind != EdgeKind::Imports {
            return None;
        }
        if let Some(rest) = target.strip_prefix("std/") {
            return Some(format!("ext:nim-stdlib:{rest}"));
        }
        if target.starts_with("std/") {
            return Some("ext:nim-stdlib".to_string());
        }
        if let Some(rest) = target.strip_prefix("pkg/") {
            return Some(format!("ext:nim-pkg:{rest}"));
        }
        None
    }
}

pub static NIM_HOOKS: NimHooks = NimHooks;
