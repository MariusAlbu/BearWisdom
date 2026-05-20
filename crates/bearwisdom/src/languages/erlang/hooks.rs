use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct ErlangHooks;

impl LanguageEngineHooks for ErlangHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if ref_ctx.extracted_ref.kind != EdgeKind::Calls {
            return None;
        }
        let target = &ref_ctx.extracted_ref.target_name;
        // Calls refs carry "name/arity" — strip the arity suffix.
        let bare = target.split('/').next().unwrap_or(target.as_str());
        if bare.is_empty() {
            return None;
        }
        let plugin_keywords = crate::indexer::keywords::keywords_for_language("erlang");
        if plugin_keywords.contains(&bare) {
            return Some("primitive".to_string());
        }
        if super::keywords::KEYWORDS.contains(&bare) {
            return Some("builtin".to_string());
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
}

pub static ERLANG_HOOKS: ErlangHooks = ErlangHooks;
