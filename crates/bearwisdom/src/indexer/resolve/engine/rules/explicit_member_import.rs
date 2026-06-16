// =============================================================================
// engine/rules/explicit_member_import — module path whose last segment IS
// the imported name
//
// Gated on `ctx.profile.explicit_member_import` (default off).
//
// For a non-dotted bare target: if the file has an import entry whose
// `imported_name` equals `target` AND whose `module_path` is a dotted path
// ending in `target`, the import statement is an explicit member import
// (Swift `import class Foundation.NSData`). In that case the target must
// be an unambiguous internal symbol — any candidate count other than 1
// declines to avoid false binds.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct ExplicitMemberImportRule;

impl LookupRule for ExplicitMemberImportRule {
    fn name(&self) -> &'static str {
        "explicit_member_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.explicit_member_import {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        // The import scope is genuine only when the import statement explicitly
        // names this symbol: a dotted module path whose last segment is the
        // imported name (== the bare target). A plain `import Foundation`
        // (no dot) never satisfies this.
        let armed = ctx.file_ctx.imports.iter().any(|import| {
            if import.imported_name != target {
                return false;
            }
            let Some(module) = import.module_path.as_deref() else {
                return false;
            };
            module.contains('.') && module.rsplit('.').next() == Some(target)
        });
        if !armed {
            return LookupResult::Pass;
        }

        let mut compatible: Vec<_> = ctx
            .lookup
            .by_name(target)
            .into_iter()
            .filter(|sym| !ctx.lookup.is_external_file(&sym.file_path))
            .filter(|sym| (ctx.kind)(edge_kind, &sym.kind))
            .collect();
        compatible.sort_by(|a, b| {
            a.qualified_name
                .cmp(&b.qualified_name)
                .then(a.kind.cmp(&b.kind))
        });
        compatible.dedup_by(|a, b| a.qualified_name == b.qualified_name && a.kind == b.kind);
        if compatible.len() != 1 {
            return LookupResult::Pass;
        }
        LookupResult::Resolved(
            ctx.resolved(compatible[0].id, "default_explicit_member_import"),
        )
    }
}

#[cfg(test)]
#[path = "explicit_member_import_tests.rs"]
mod tests;
