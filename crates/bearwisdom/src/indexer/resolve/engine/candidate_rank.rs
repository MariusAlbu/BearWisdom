// =============================================================================
// engine/candidate_rank — rank same-simple-name candidates by use-site evidence
//
// The single chokepoint every rung funnels through when several declarations
// share one simple name. Each term weighs one fact about the use site (its
// declared namespace, its build unit, its imports, its path) against the
// candidate row; the pick commits only when the winner clears `RANK_MARGIN`,
// so an ambiguous field declines instead of guessing.
// =============================================================================

use crate::indexer::resolve::engine::contract::{FileContext, Symbol, SymbolLookup};
use crate::type_checker::profile::language_profile::LanguageProfile;

use super::support::{qname_under_module, symbol_under_index_namespace};

/// Minimum score margin the top candidate must beat the runner-up by for
/// `pick_ranked_candidate` to commit; a smaller margin is too ambiguous to guess.
pub(crate) const RANK_MARGIN: i32 = 100;

/// An import line whose module path names the candidate's own index qname, or
/// the namespace it is declared under, is the use site stating which
/// same-simple-name declaration it means. Weighed like the workspace-package
/// term: both are an import naming the candidate's home.
const IMPORT_NAMES_CANDIDATE: i32 = 500;

/// The use site's own namespace outranks every build-unit and import term:
/// an unqualified name resolves against the enclosing namespace before any
/// import machinery runs. Outweighs the same-build-unit term because a source
/// namespace is stronger evidence than a build artifact — a caller in `a.b` with
/// homonyms `a.b.Foo` (another build unit) and `a.c.Foo` (the caller's own unit)
/// binds the one its namespace names.
const SAME_SOURCE_NAMESPACE: i32 = 1200;

/// Score a same-name candidate for import-scoped selection — higher is better.
/// Reads only the candidate row, the use site's file context (its namespace,
/// package, imports, path) and the language's profile: the file's own declared
/// namespace +1200, same workspace package +1000, an import naming the
/// candidate's package or its own namespace +500, an implicit namespace the
/// manifest or the language prelude brings into scope +300, ambient +200,
/// shared path prefix +10/segment, an external-depth penalty, and a
/// visibility hint.
pub(crate) fn score_candidate(
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    sym: &Symbol,
) -> i32 {
    let profile = crate::languages::default_registry().profile_for(&file_ctx.language);
    let mut s: i32 = 0;
    // Exact equality only: an enclosing/parent namespace earns nothing, so a
    // language whose visibility reaches outward does not get a silent free win.
    if let Some(ns) = file_ctx.file_namespace.as_deref().filter(|ns| !ns.is_empty()) {
        if sym.scope_path.as_deref() == Some(ns) {
            s += SAME_SOURCE_NAMESPACE;
        }
    }
    if let (Some(caller_pkg), Some(sym_pkg)) = (file_package_id, sym.package_id) {
        if caller_pkg == sym_pkg {
            s += 1000;
        }
    }
    for import in &file_ctx.imports {
        let Some(mod_path) = import.module_path.as_deref() else {
            continue;
        };
        let mod_path = profile.workspace_specifier_path(mod_path);
        if let Some(wp_id) = lookup.workspace_package_id(mod_path.as_ref()) {
            if Some(wp_id) == sym.package_id {
                s += 500;
            }
        }
    }
    // Scored once per candidate: the evidence is that an import names it, not
    // how many imports do.
    if any_import_names(file_ctx, profile, sym) {
        s += IMPORT_NAMES_CANDIDATE;
    }
    // An implicit/global wildcard namespace scopes a candidate exactly like a
    // written namespace import: the file sees that namespace's members with no
    // import line, so a candidate under it outranks a same-named type no scope
    // names. Two sources declare one: the manifest or SDK
    // (`implicit_wildcard_namespaces`) and the language's own compiler prelude
    // (`LanguageProfile::implicit_prelude_namespaces`).
    let implicit = lookup
        .implicit_wildcard_namespaces(file_package_id)
        .iter()
        .map(String::as_str)
        .chain(profile.implicit_prelude_namespaces.iter().copied());
    for ns in implicit {
        if symbol_under_index_namespace(sym, ns) {
            s += 300;
            break;
        }
    }
    if lookup.is_ambient_path(&sym.file_path) {
        s += 200;
    }
    s += path_proximity_score(&file_ctx.file_path, &sym.file_path);
    if lookup.is_external_file(&sym.file_path) {
        let depth = sym.file_path.matches('/').count() as i32;
        s -= depth.min(20);
    }
    match sym.visibility.as_deref() {
        Some("public") => s += 50,
        Some("private") => s -= 200,
        _ => {}
    }
    s
}

/// `true` when some import line's module path names `sym`: either the
/// candidate's own index qname (a single-type import) or the namespace it is
/// declared under (a namespace import). The profile owns the source-to-index
/// spelling of the module path.
fn any_import_names(file_ctx: &FileContext, profile: &LanguageProfile, sym: &Symbol) -> bool {
    file_ctx.imports.iter().any(|import| {
        import
            .module_path
            .as_deref()
            .is_some_and(|module| qname_under_module(profile, &sym.qualified_name, module))
    })
}

/// The shared import-scoped candidate chokepoint. Returns the top scorer when it
/// beats the runner-up by `RANK_MARGIN`, the sole candidate when there is one, or
/// `None` when the field is empty or too ambiguous to commit (the caller keeps
/// its own fallback). Deterministic: equal scores break by ascending id, so the
/// pick never depends on candidate insertion order.
pub(crate) fn pick_ranked_candidate<'a>(
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    candidates: &[&'a Symbol],
) -> Option<&'a Symbol> {
    match candidates.len() {
        0 => return None,
        1 => return Some(candidates[0]),
        _ => {}
    }
    let mut scored: Vec<(i32, &'a Symbol)> = candidates
        .iter()
        .map(|sym| {
            (
                score_candidate(file_ctx, file_package_id, lookup, sym),
                *sym,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    let (top_score, top) = scored[0];
    let (runner_score, _) = scored[1];
    if top_score - runner_score < RANK_MARGIN {
        return None;
    }
    Some(top)
}

/// Shared-directory-prefix score between two file paths: 10 × the number of
/// shared leading directory segments. Separators normalise to `/`; the filename
/// is dropped before comparing.
fn path_proximity_score(caller_path: &str, candidate_path: &str) -> i32 {
    let caller_norm = caller_path.replace('\\', "/");
    let candidate_norm = candidate_path.replace('\\', "/");
    let caller_dir = caller_norm.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let candidate_dir = candidate_norm
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("");
    let caller_segs: Vec<&str> = caller_dir.split('/').filter(|s| !s.is_empty()).collect();
    let candidate_segs: Vec<&str> = candidate_dir.split('/').filter(|s| !s.is_empty()).collect();
    caller_segs
        .iter()
        .zip(candidate_segs.iter())
        .take_while(|(a, b)| a == b)
        .count() as i32
        * 10
}

#[cfg(test)]
#[path = "candidate_rank_tests.rs"]
mod tests;
