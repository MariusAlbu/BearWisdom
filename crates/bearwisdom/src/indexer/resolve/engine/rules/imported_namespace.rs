// =============================================================================
// engine/rules/imported_namespace — qname is prefixed by an imported
// namespace
//
// `using FamilyBudget.Api.Entities;` then `Transaction`: the candidate's qname
// `FamilyBudget.Api.Entities.Transaction` starts with the imported namespace.
// A boundary check ensures the prefix doesn't accidentally match a longer
// namespace (`FamilyBudget.Api.EntitiesOther`). A path-based fallback accepts
// candidates whose file path matches the module string, covering package-
// directory imports.
//
// `candidate_namespace_prefixes` is copied inline because it is used by both
// this rule and `namespace_import`, but the two rules are non-identical:
// `namespace_import` requires a `.` in the prefix; this rule scans by_name +
// file_path_matches_module instead.
//
// The local `file_path_matches_module` widens the shared matcher (see
// `engine/path_match`) with a hyphen-to-underscore folded segment-run retry.
// =============================================================================

use crate::indexer::resolve::engine::support::{
    file_path_matches_module as shared_file_path_matches_module, import_scoped_package_id,
    normalize_name,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::NameNormalization;

pub struct ImportedNamespaceRule;

impl LookupRule for ImportedNamespaceRule {
    fn name(&self) -> &'static str {
        "imported_namespace"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;

        // Import-scoped preference: when the file imports `target` from a bare
        // sibling-workspace specifier, bind the same-named candidate in THAT
        // package before falling through to the first by-name match. Claims the
        // same-name-across-packages refs the byte-exact probe would mis-route.
        if let Some(pkg) = import_scoped_package_id(ctx.file_ctx, ctx.lookup, target) {
            for sym in ctx.lookup.by_name(target) {
                if sym.package_id == Some(pkg) && (ctx.kind)(edge_kind, &sym.kind) {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_imported_namespace"),
                    );
                }
            }
        }

        // Byte-exact probe: scan by_name candidates and check whether any
        // import's module path is a prefix of the candidate's qualified name.
        for sym in ctx.lookup.by_name(target) {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            for import in &ctx.file_ctx.imports {
                let Some(module) = &import.module_path else {
                    continue;
                };
                // A RELATIVE import resolves inside the importing package's
                // own tree — an `ext:` candidate can never be its target,
                // whatever its stem looks like (`./types` must not match an
                // unrelated dependency's `…/internal/types.d.ts`).
                if ctx
                    .profile
                    .source_module_path_policy(module)
                    .is_relative(module)
                    && sym.file_path.starts_with("ext:")
                {
                    continue;
                }
                let indexed_module = ctx.profile.index_qname_from_source(module);
                if sym.qualified_name.starts_with(&indexed_module) {
                    let rest = &sym.qualified_name[indexed_module.len()..];
                    if rest.is_empty() || rest.starts_with('.') {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_imported_namespace"),
                        );
                    }
                }
                if file_path_matches_module(&sym.file_path, module, ctx.profile) {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_imported_namespace"),
                    );
                }
            }
        }

        // Case-folding fallback: under a folding spec the candidate's declared
        // leaf may differ in case from `target`, so `by_name(target)` above
        // misses. For each imported module, scan its members and accept one
        // whose qname folds equal to `{module}.{target}`.
        if !matches!(norm, NameNormalization::None) {
            for import in &ctx.file_ctx.imports {
                let Some(module) = &import.module_path else {
                    continue;
                };
                let expected = ctx.profile.index_qname_join(module, target);
                let expected_norm = normalize_name(norm, &expected);
                let indexed_module = ctx.profile.index_qname_from_source(module);
                for sym in ctx.lookup.in_namespace(&indexed_module) {
                    if normalize_name(norm, &sym.qualified_name) == expected_norm
                        && (ctx.kind)(edge_kind, &sym.kind)
                    {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_imported_namespace"),
                        );
                    }
                }
            }
        }

        LookupResult::Pass
    }
}

/// Returns `true` when `file_path` resolves to the module string. Both a stem
/// suffix match and a package-directory segment-run match are checked.
///
/// Hyphen/underscore normalization: many ecosystems use hyphens in on-disk
/// directory names while the import statement uses underscores (e.g. Rust's
/// `use turbo_tasks::Vc` where the crate directory is `turbo-tasks/`). After
/// the literal segment-run check fails, both path and run are re-checked with
/// hyphens folded to underscores. The segment-boundary requirement still
/// applies after normalization, so `turbo_tasks_macros` never matches `turbo_tasks`.
fn file_path_matches_module(
    file_path: &str,
    module: &str,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> bool {
    if shared_file_path_matches_module(file_path, module, profile) {
        return true;
    }
    (profile
        .source_module_path_policy(module)
        .bare_module_matches_file)(file_path, module)
}

#[cfg(test)]
#[path = "imported_namespace_tests.rs"]
mod tests;
