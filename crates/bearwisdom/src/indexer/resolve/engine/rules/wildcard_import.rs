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
//   PackageRoot — the candidate's EXTERNAL file's `ext:<lang>:<pkg>/…`
//                 package segment matches the module name; falls back to the
//                 same FileStem check for an internal candidate or a module
//                 with no package identity, so it's a strict superset of
//                 FileStem for a language whose wildcards mix package-scheme
//                 imports (barrel-defeating) with plain relative ones.
//
// Fires only when at least one wildcard import is present.  A dotted or
// `::` target is declined — qualified refs are handled earlier in the ladder.
// Accepts only when EXACTLY ONE candidate matches — ambiguity stays unresolved.
//
// `wildcard_file_stem_matches` / `wildcard_package_segment` are inlined here;
// both are specific to this rule.
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

        let mut wildcards: Vec<&str> = ctx
            .file_ctx
            .imports
            .iter()
            .filter(|imp| imp.is_wildcard)
            .filter_map(|imp| imp.module_path.as_deref())
            .filter(|m| !m.is_empty())
            .collect();
        // Manifest-declared implicit/global namespaces (`<ImplicitUsings>`,
        // `<Using Include="X">`) open the same bare scope as a written
        // namespace import — files under those SDKs carry no `using` line for
        // them at all. Gated with the namespace-wildcard opt-in: a language
        // whose plain imports aren't wildcards has no implicit-namespace
        // semantics either.
        if ctx.profile.namespace_imports_are_wildcards {
            wildcards.extend(
                ctx.lookup
                    .implicit_wildcard_namespaces(ctx.ref_ctx.file_package_id)
                    .iter()
                    .map(String::as_str),
            );
        }
        if wildcards.is_empty() {
            return LookupResult::Pass;
        }

        let target_norm = normalize_name(norm, target);
        // Hits are keyed by QUALIFIED NAME: one declaration surfaced as
        // several same-qname rows (arity overloads, partials, merged decls)
        // is ONE unambiguous hit. Two DIFFERENT qnames stay ambiguous.
        let mut hit_qname: Option<String> = None;
        let mut hit_id: Option<i64> = None;
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
                WildcardMatch::PackageRoot => {
                    if normalize_name(norm, &sym.name) != target_norm {
                        false
                    } else {
                        let is_external = ctx.lookup.is_external_file(&sym.file_path);
                        let pkg_seg = is_external
                            .then(|| wildcard_package_segment(&sym.file_path))
                            .filter(|p| !p.is_empty());
                        let file_lower = sym.file_path.to_lowercase();
                        wildcards.iter().any(|ns| {
                            if let Some(pkg) = pkg_seg {
                                if normalize_name(norm, pkg) == normalize_name(norm, ns) {
                                    return true;
                                }
                            }
                            wildcard_file_stem_matches(&file_lower, &ns.to_lowercase(), false)
                        })
                    }
                }
            };
            if under_a_wildcard {
                match &hit_qname {
                    None => {
                        hit_qname = Some(sym.qualified_name.clone());
                        hit_id = Some(sym.id);
                    }
                    Some(q) if *q == sym.qualified_name => {}
                    // A second DISTINCT qname — ambiguous; stay unresolved and
                    // let ranked_candidates decide later.
                    Some(_) => return LookupResult::Pass,
                }
            }
        }
        if let Some(id) = hit_id {
            return LookupResult::Resolved(ctx.resolved(id, "default_wildcard_import"));
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

/// The package segment of a candidate's `ext:<lang>:<pkg>/…` virtual path —
/// the same three-colon convention `ExternalByImportRule` reads for
/// `ExtMatch::PkgSegment`. Empty for a path that doesn't match the shape,
/// including every internal (non-`ext:`) path.
fn wildcard_package_segment(path: &str) -> &str {
    let Some(rest) = path.strip_prefix("ext:") else {
        return "";
    };
    let Some((_lang, after_lang)) = rest.split_once(':') else {
        return "";
    };
    after_lang.split('/').next().unwrap_or("")
}

#[cfg(test)]
#[path = "wildcard_import_tests.rs"]
mod tests;
