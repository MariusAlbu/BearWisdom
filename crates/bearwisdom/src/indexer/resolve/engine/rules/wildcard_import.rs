// =============================================================================
// engine/rules/wildcard_import — bare name under a wildcard import module
//
// A wildcard import (`use foo::*`, `from m import *`, `using namespace X`)
// brings every DIRECT member of a namespace or every symbol in a file into
// scope.  Two dispatch modes:
//
//   QnameUnder  — the candidate's qname is exactly one segment deeper than the
//                 wildcard's module path.  Default.
//   FileStem    — the candidate's FILE basename-stem or a path dir-segment
//                 matches the module name under `name_normalization`, with an
//                 optional `{stem}_`-prefixed include-file probe.
//
// Fires only when at least one wildcard import is present.  A dotted or
// `::` target is declined — qualified refs are handled earlier in the ladder.
// Accepts only when EXACTLY ONE candidate matches — ambiguity stays unresolved.
//
// `wildcard_file_stem_matches` is inlined here; it is specific to this rule.
// =============================================================================

use crate::indexer::resolve::engine::support::{
    basename_stem_matches, normalize_name, qname_directly_under,
};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::WildcardMatch;

pub struct WildcardImportRule;

impl LookupRule for WildcardImportRule {
    fn name(&self) -> &'static str {
        "wildcard_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let mode = ctx.profile.wildcard_match;
        let norm = ctx.profile.name_normalization;

        let wildcards: Vec<&str> = ctx
            .file_ctx
            .imports
            .iter()
            .filter(|imp| imp.is_wildcard)
            .filter_map(|imp| imp.module_path.as_deref())
            .filter(|m| !m.is_empty())
            .collect();
        if wildcards.is_empty() {
            return LookupResult::Pass;
        }

        let target_norm = normalize_name(norm, target);
        let mut hits: Vec<i64> = Vec::new();
        for sym in ctx.lookup.by_name(target) {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            let under_a_wildcard = match mode {
                WildcardMatch::QnameUnder => wildcards
                    .iter()
                    .any(|ns| qname_directly_under(&sym.qualified_name, ns)),
                WildcardMatch::FileStem { underscore_prefix } => {
                    if normalize_name(norm, &sym.name) != target_norm {
                        false
                    } else {
                        let file_lower = sym.file_path.to_lowercase();
                        wildcards.iter().any(|ns| {
                            let ns_lower = ns.to_lowercase();
                            wildcard_file_stem_matches(&file_lower, &ns_lower, underscore_prefix)
                        })
                    }
                }
            };
            if under_a_wildcard {
                hits.push(sym.id);
            }
        }
        // Single hit — accept. Multiple — stay unresolved; let ranked_candidates
        // decide later.
        if hits.len() == 1 {
            return LookupResult::Resolved(ctx.resolved(hits[0], "default_wildcard_import"));
        }
        LookupResult::Pass
    }
}

/// `true` when the candidate file's basename-stem matches the module name, or
/// when `underscore_prefix` is set and the stem begins with `{module}_`.
/// Both `file_path_lower` and `module_lower` are already lowercased by the
/// caller.
fn wildcard_file_stem_matches(
    file_path_lower: &str,
    module_lower: &str,
    underscore_prefix: bool,
) -> bool {
    if basename_stem_matches(file_path_lower, module_lower) {
        return true;
    }
    if !underscore_prefix || module_lower.is_empty() {
        return false;
    }
    let normalized = file_path_lower.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(&normalized);
    let stem = basename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(basename);
    stem.starts_with(&format!("{module_lower}_"))
}

#[cfg(test)]
#[path = "wildcard_import_tests.rs"]
mod tests;
