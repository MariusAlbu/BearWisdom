use super::resolve;
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

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        resolve::detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }
}

pub static NIM_HOOKS: NimHooks = NimHooks;
