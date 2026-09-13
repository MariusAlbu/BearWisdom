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
// Fires only when at least one wildcard import is present. A target qualified
// with the profile's source separator is declined —
// qualified refs are handled earlier in the ladder.
// Accepts only when EXACTLY ONE candidate matches — ambiguity stays unresolved.
//
// A wildcard's module is the import's own spelling. The file stem it names and
// its external package identity come from the ecosystem package-specifier
// adapter; a language without one compares the spelling itself.
// =============================================================================

use crate::ecosystem::package_specifier::{import_file_stem, import_package_root};
use crate::indexer::resolve::engine::support::{
    basename_stem_matches, normalize_name, qname_directly_under,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::WildcardMatch;

pub struct WildcardImportRule;

impl LookupRule for WildcardImportRule {
    fn name(&self) -> &'static str {
        "wildcard_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() || ctx.profile.is_qualified_name(target) {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let mode = ctx.profile.imports.wildcard_match;
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
        if ctx.profile.imports.namespace_imports_are_wildcards {
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

        // Per wildcard: the file stem the spelling names and its package
        // identity, as the owning ecosystem reads them.
        let language = ctx.file_ctx.language.as_str();
        let stems: Vec<String> = wildcards
            .iter()
            .map(|ns| import_file_stem(language, ns).unwrap_or_else(|| (*ns).to_string()))
            .collect();
        let roots: Vec<String> = wildcards
            .iter()
            .map(|ns| import_package_root(language, ns).unwrap_or_else(|| (*ns).to_string()))
            .collect();
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
                WildcardMatch::QnameUnder | WildcardMatch::QnameUnderWithPhysicalFiles { .. } => {
                    wildcards
                        .iter()
                        .any(|ns| qname_directly_under(ctx.profile, &sym.qualified_name, ns))
                }
                WildcardMatch::FileStem { underscore_prefix } => {
                    if normalize_name(norm, &sym.name) != target_norm {
                        false
                    } else {
                        let file_lower = sym.file_path.to_lowercase();
                        stems.iter().any(|stem| {
                            let stem_lower = stem.to_lowercase();
                            wildcard_file_stem_matches(&file_lower, &stem_lower, underscore_prefix)
                        })
                    }
                }
                WildcardMatch::PackageRoot => {
                    if normalize_name(norm, &sym.name) != target_norm {
                        false
                    } else {
                        let is_external = ctx.lookup.is_external_file(&sym.file_path);
                        let pkg_seg = is_external
                            .then(|| {
                                crate::ecosystem::package_specifier::external_package_key(
                                    &ctx.file_ctx.language,
                                    &sym.file_path,
                                )
                            })
                            .flatten();
                        let file_lower = sym.file_path.to_lowercase();
                        roots.iter().zip(&stems).any(|(root, stem)| {
                            if let Some(pkg) = pkg_seg.as_deref() {
                                if normalize_name(norm, pkg) == normalize_name(norm, root) {
                                    return true;
                                }
                            }
                            wildcard_file_stem_matches(&file_lower, &stem.to_lowercase(), false)
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

#[cfg(test)]
#[path = "wildcard_import_tests.rs"]
mod tests;
