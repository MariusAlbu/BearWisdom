// =============================================================================
// engine/rules/aliased_import — import specifier resolved via path alias
//
// `resolve_via_file_import` matches an import's `module_path` directly against
// candidate file paths. This rule handles specifiers that are path aliases
// (`@/utils`, `$lib/...`) which must first be rewritten to a real path.
// Fires only when the rewrite changes the specifier — the raw-path case already
// ran in `file_import`.
// =============================================================================

use crate::indexer::resolve::engine::support::file_path_matches_module;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct AliasedImportRule;

impl LookupRule for AliasedImportRule {
    fn name(&self) -> &'static str {
        "aliased_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        for import in &ctx.file_ctx.imports {
            let matches_direct = import.imported_name == target;
            let matches_alias = import.alias.as_deref() == Some(target);
            if !matches_direct && !matches_alias {
                continue;
            }
            let Some(raw_module) = import.module_path.as_deref() else {
                continue;
            };
            let Some(rewritten) = ctx
                .lookup
                .resolve_path_alias(ctx.ref_ctx.file_package_id, raw_module)
            else {
                continue;
            };
            if rewritten == raw_module {
                continue;
            }
            let lookup_name = if matches_alias {
                import.imported_name.as_str()
            } else {
                target
            };
            for sym in ctx.lookup.by_name(lookup_name) {
                if (ctx.kind)(edge_kind, &sym.kind)
                    && file_path_matches_module(&sym.file_path, &rewritten)
                {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "engine_aliased_import"),
                    );
                }
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "aliased_import_tests.rs"]
mod tests;
