use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{self as engine, FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct ZigHooks;

impl LanguageEngineHooks for ZigHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if let Some(ns) =
            engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_zig_builtin)
        {
            // Common returns "builtin"; remap to zig's "zig.builtin" label.
            return Some(if ns == "builtin" { "zig.builtin".to_string() } else { ns });
        }
        None
    }
}

pub static ZIG_HOOKS: ZigHooks = ZigHooks;
