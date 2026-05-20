use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct NixHooks;

impl LanguageEngineHooks for NixHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Channel refs like <nixpkgs> are external; relative path imports
        // are local and must NOT be marked external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if path.starts_with('<') && path.ends_with('>') {
                return Some(path.to_string());
            }
            return None;
        }

        // Dotted platform attribute paths are always external.
        if target.starts_with("builtins.")
            || target.starts_with("lib.")
            || target.starts_with("pkgs.")
            || target.starts_with("config.")
        {
            return Some("builtin".to_string());
        }
        None
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
        super::resolve::NixResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static NIX_HOOKS: NixHooks = NixHooks;
