// =============================================================================
// engine/rules/aliased_import — import specifier resolved via path alias
//
// `resolve_via_file_import` matches an import's `module_path` directly against
// candidate file paths. This rule handles specifiers that are path aliases
// (`@/utils`, `$lib/...`) which must first be rewritten to a real path.
// Fires only when the rewrite changes the specifier — the raw-path case already
// ran in `file_import`.
// =============================================================================

use crate::indexer::resolve::engine::support::{trim_path_extension, trim_source_extension};
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

/// Match a symbol's file path against a (rewritten) module specifier. The same
/// multi-form matcher as in `file_import`: stem-suffix, dot-to-slash rewrite,
/// and segment-bounded package-directory run.
fn file_path_matches_module(file_path: &str, module: &str) -> bool {
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let cleaned =
        trim_source_extension(module.trim_start_matches("./").trim_start_matches("../"));
    let stem = trim_path_extension(&normalized);
    if stem.ends_with(cleaned) || stem.ends_with(&cleaned.replace('.', "/")) {
        return true;
    }
    let dotted = cleaned.replace('.', "/");
    if dotted.is_empty() {
        return false;
    }
    path_contains_segment_run(&normalized, &dotted)
}

fn path_contains_segment_run(path: &str, run: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = path[from..].find(run) {
        let start = from + rel;
        let end = start + run.len();
        let left_ok = start == 0 || path.as_bytes()[start - 1] == b'/';
        let right_ok = end == path.len() || path.as_bytes()[end] == b'/';
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[cfg(test)]
#[path = "aliased_import_tests.rs"]
mod tests;
