// =============================================================================
// engine/rules/package_short_name — import names a PACKAGE; member keyed
// under the import's short name
//
// Go: `import "github.com/gin-gonic/gin"` brings short name `gin`; a function
// call `NewRouter` is stored as `gin.NewRouter`. For each import, derive the
// short name as the trailing path segment (split on `/`) and then on `sep`
// (for profiles whose qualified paths have their own separator). Probe both the
// import's `imported_name` and the `last_path_segment` under `{prefix}{sep}{target}`.
//
// GATED: only fires when `ctx.profile.chain_qualification ==
// ChainQualification::PackageShortName`. All other languages return `Pass`
// immediately, leaving the ordinary bare-name path to handle them.
//
// `sep` is the last of the scope-visible separator slice — identical to
// the active profile's source-name normalization,
// and `"."` for `.`-separator languages. Mirrored from the run_ladder call
// site: `separators.last().copied().unwrap_or(".")`.
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::ChainQualification;

pub struct PackageShortNameRule;

impl LookupRule for PackageShortNameRule {
    fn name(&self) -> &'static str {
        "package_short_name"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        // Gate: only active for package-short-name import shapes.
        if !matches!(
            ctx.profile.chain_qualification,
            ChainQualification::PackageShortName
        ) {
            return LookupResult::Pass;
        }

        let target = ctx.target();
        if target.is_empty() || ctx.profile.is_qualified_name(target) {
            return LookupResult::Pass;
        }

        let edge_kind = ctx.edge_kind();

        for import in &ctx.file_ctx.imports {
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            let last_seg = ctx.profile.simple_name(module);
            for prefix in [import.imported_name.as_str(), last_seg] {
                if prefix.is_empty() {
                    continue;
                }
                let qname = ctx.profile.index_qname_join(prefix, target);
                if let Some(sym) = ctx.lookup.by_qualified_name(&qname) {
                    if (ctx.kind)(edge_kind, &sym.kind) {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_package_short_name"),
                        );
                    }
                }
            }
        }

        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "package_short_name_tests.rs"]
mod tests;
