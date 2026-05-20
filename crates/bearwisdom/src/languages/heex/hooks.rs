use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
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
}

pub static HEEX_HOOKS: HeexHooks = HeexHooks;
