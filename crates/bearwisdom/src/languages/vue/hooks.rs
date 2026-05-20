use super::global_registry;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct VueHooks;

impl LanguageEngineHooks for VueHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Suppress external classification for explicitly globally-registered
        // PascalCase components (app.component) — these resolve by name in the
        // project index.
        if let Some(ctx_ref) = project_ctx {
            if let Some(registry) = ctx_ref.plugin_state.get::<global_registry::VueGlobalRegistry>() {
                let name = &ref_ctx.extracted_ref.target_name;
                if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    if let Some(global_registry::VueComponentSource::ExplicitRegistration { .. }) =
                        registry.components.get(name.as_str())
                    {
                        return None;
                    }
                }
            }
        }
        crate::languages::typescript::resolve::infer_external_inner_with_lookup(
            file_ctx, ref_ctx, project_ctx, lookup,
        )
    }
}

pub static VUE_HOOKS: VueHooks = VueHooks;
