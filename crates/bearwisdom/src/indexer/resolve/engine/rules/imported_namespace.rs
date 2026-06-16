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
// `file_path_matches_module` and `path_contains_segment_run` are single-use
// helpers inlined here from `default_resolver`.
// =============================================================================

use crate::indexer::resolve::engine::support::normalize_name;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
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
                if sym.qualified_name.starts_with(module.as_str()) {
                    let rest = &sym.qualified_name[module.len()..];
                    if rest.is_empty() || rest.starts_with('.') {
                        return LookupResult::Resolved(
                            ctx.resolved(sym.id, "default_imported_namespace"),
                        );
                    }
                }
                if file_path_matches_module(&sym.file_path, module) {
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
                let expected = format!("{module}.{target}");
                let expected_norm = normalize_name(norm, &expected);
                for sym in ctx.lookup.in_namespace(module) {
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
fn file_path_matches_module(file_path: &str, module: &str) -> bool {
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let cleaned =
        trim_source_extension(module.trim_start_matches("./").trim_start_matches("../"));
    let stem = trim_source_extension(&normalized);
    if stem.ends_with(cleaned) || stem.ends_with(&cleaned.replace('.', "/")) {
        return true;
    }
    let dotted = cleaned.replace('.', "/");
    if dotted.is_empty() {
        return false;
    }
    path_contains_segment_run(&normalized, &dotted)
}

/// Returns `true` when `run` appears as a contiguous, segment-bounded
/// substring of `path` (bounded by `/` or string start/end).
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

/// Trim known source-file extensions for stem comparison.
fn trim_source_extension(path: &str) -> &str {
    path.trim_end_matches(".svelte")
        .trim_end_matches(".vue")
        .trim_end_matches(".tsx")
        .trim_end_matches(".jsx")
        .trim_end_matches(".mts")
        .trim_end_matches(".cts")
        .trim_end_matches(".ts")
        .trim_end_matches(".js")
        .trim_end_matches(".cs")
        .trim_end_matches(".cljc")
        .trim_end_matches(".cljs")
        .trim_end_matches(".clj")
        .trim_end_matches(".astro")
        .trim_end_matches(".mdx")
}

#[cfg(test)]
#[path = "imported_namespace_tests.rs"]
mod tests;
