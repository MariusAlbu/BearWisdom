use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::languages::elixir;
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct HeexHooks;

impl LanguageEngineHooks for HeexHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        // Module-qualified component refs like `<MyApp.Components.button>` —
        // delegate to the Elixir external-module classifier.
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);
            if elixir::predicates::is_external_elixir_module(root) {
                return Some(root.to_string());
            }
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
        super::resolve::HeexResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static HEEX_HOOKS: HeexHooks = HeexHooks;
