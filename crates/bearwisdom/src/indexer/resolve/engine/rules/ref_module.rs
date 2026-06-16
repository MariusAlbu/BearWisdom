// =============================================================================
// engine/rules/ref_module — module-qualified resolution via `r.module`
//
// The extractor recorded an explicit module prefix on the ref —
// Erlang `lists:map`, OCaml `List.map`, R `dplyr::mutate`. Tries:
// (a) `module{sep}target` as an exact qualified name under each separator
//     (`.`, `::`, `/`, `:`), then
// (b) any `target` candidate whose file-path stem matches the module name.
//
// Declines immediately when no `module` field is set on the ref.
// =============================================================================

use crate::indexer::resolve::engine::support::path_stem_matches;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct RefModuleRule;

impl LookupRule for RefModuleRule {
    fn name(&self) -> &'static str {
        "ref_module"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        let Some(module) = ctx.r().module.as_deref() else {
            return LookupResult::Pass;
        };

        for sep in [".", "::", "/", ":"] {
            let qname = format!("{module}{sep}{target}");
            if let Some(sym) = ctx.lookup.by_qualified_name(&qname) {
                if (ctx.kind)(edge_kind, &sym.kind) {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "default_ref_module"));
                }
            }
        }

        let module_lower = module.to_lowercase();
        let last_seg_lower = module.rsplit('.').next().unwrap_or(module).to_lowercase();
        for sym in ctx.lookup.by_name(target) {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            let file_lower = sym.file_path.to_lowercase();
            if path_stem_matches(&file_lower, &module_lower)
                || path_stem_matches(&file_lower, &last_seg_lower)
            {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_ref_module"));
            }
        }

        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "ref_module_tests.rs"]
mod tests;
