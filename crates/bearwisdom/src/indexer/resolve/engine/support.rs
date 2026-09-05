// =============================================================================
// indexer/resolve/engine/support — helpers shared by multiple rules
//
// Only code that genuinely repeats across rule files lives here. A helper used
// by a single rule is copied inline into that rule's file instead, so each rule
// reads top-to-bottom without chasing this module. Everything here is a pure
// function of its inputs.
// =============================================================================

use std::borrow::Cow;
use std::collections::BTreeMap;

use rustc_hash::FxHashMap;

pub(crate) use super::path_match::{
    basename_stem_matches, file_path_matches_module, is_bare_module_specifier,
    is_relative_specifier, parent_dir, path_contains_segment_run, path_stem_matches,
    trim_path_extension, trim_source_extension,
};

pub(crate) use super::reexports::{
    follow_reexports, relative_reexport_candidates, workspace_pkg_barrels,
    workspace_pkg_declared_symbol,
};

use crate::indexer::resolve::engine::contract::{
    FileContext, Symbol, SymbolInfo, SymbolLookup, TypeInfo, RESOLVED_CONFIDENCE,
};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::type_checker::profile::language_profile::{LanguageProfile, NameNormalization, NormSpec};
use crate::types::EdgeKind;

/// Resolve a module-tagged value `TypeRef` to the type of the value module
/// `module` exports as `key`. A module-tagged value ref is what
/// `typeof import('m')['k']` (or `typeof import('m')`) annotations produce: the
/// ref means "the type of the value exported as `k` from `m`", NOT a type name —
/// so it must never be looked up as a bare type name against the scope.
///
/// The exported value's declaring symbol is reached two ways: a local export
/// rename (`export { local as k }`, captured in `export_alias`) maps `k` to the
/// local declaration's qname; otherwise the direct export `m.k` is the
/// declaration. The declaration's own `field_type` / `field_type_id` is the
/// resolved type (`globalV: I` → `I`).
///
/// `typed_qname` is the symbol the ref is being resolved FOR; a declaration that
/// resolves back to it is the self-referential `m.k` case and is declined so the
/// caller falls back rather than typing a value as itself. Returns the resolved
/// `(field_type, field_type_id)`, or `None` when the export or its type can't be
/// resolved.
pub(crate) fn resolve_module_exported_value_type(
    module: &str,
    key: &str,
    typed_qname: &str,
    export_alias: &FxHashMap<String, FxHashMap<String, String>>,
    by_qname: &BTreeMap<String, Symbol>,
    type_info: &FxHashMap<String, TypeInfo>,
    arena: &TypeArena,
) -> Option<TypeId> {
    let declaring_qname = export_alias
        .get(module)
        .and_then(|aliases| aliases.get(key))
        .cloned()
        .or_else(|| {
            let direct = format!("{module}.{key}");
            by_qname.contains_key(&direct).then_some(direct)
        })?;
    if declaring_qname == typed_qname {
        return None;
    }
    // A `typeof <value>` whose declared value is a function/method: the value IS
    // that callable, so its type is the function itself — a call on the typed
    // property then walks through the callable's return_type_id.
    if by_qname
        .get(&declaring_qname)
        .is_some_and(|s| matches!(s.kind.as_str(), "function" | "method"))
    {
        return Some(arena.class(&declaring_qname));
    }
    let ti = type_info.get(&declaring_qname)?;
    ti.field_type_id
}

/// The workspace package id the file imports `name` from, when the import's
/// module specifier is a bare specifier that resolves to a sibling workspace
/// package. `None` when nothing imports `name`, the binding specifier is
/// relative, or no workspace package declares it.
///
/// Lets a bare reference to a name that exists in several sibling packages bind
/// to the package the use site actually imports it from, instead of a first-wins
/// same-name pick. The caller filters its own candidate set by the returned id.
pub(crate) fn import_scoped_package_id(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    name: &str,
) -> Option<i64> {
    for import in &file_ctx.imports {
        let names_target =
            import.imported_name == name || import.alias.as_deref() == Some(name);
        if !names_target {
            continue;
        }
        let Some(spec) = import.module_path.as_deref() else {
            continue;
        };
        if let Some(pkg) = lookup.workspace_package_id(spec) {
            return Some(pkg);
        }
    }
    None
}

/// The sub-path remainder after the longest declared workspace-package prefix
/// that `specifier` starts with. `None` when `specifier` IS a declared name
/// (no sub-path) or when no workspace package matches.
///
/// Peels on `/`, treating a `::`-qualified specifier (Rust's
/// `tantivy::schema`) the same way — `::` is canonicalized to `/` first, the
/// same normalization `workspace_package_id` applies, so both agree on the
/// same package root and sub-path for a given specifier.
pub(crate) fn workspace_sub_path(specifier: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    let normalized;
    let specifier: &str = if specifier.contains("::") {
        normalized = specifier.replace("::", "/");
        &normalized
    } else {
        specifier
    };
    if lookup.is_workspace_declared_name(specifier) {
        return None;
    }
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if lookup.is_workspace_declared_name(path) {
            return Some(specifier[path.len() + 1..].to_string());
        }
    }
    None
}

/// When `specifier`'s leading segment is `profile.imports.self_package_root`
/// (Rust's `crate`), the sub-path remainder that follows it: `Some(None)` for
/// the bare keyword (`crate`), `Some(Some(rest))` for a deeper path
/// (`crate::thing` -> `Some(Some("thing"))`). `None` when the profile carries
/// no such keyword or `specifier` doesn't lead with it, so the caller falls
/// back to the declared-name lookup.
pub(crate) fn self_package_sub_path(
    profile: &LanguageProfile,
    specifier: &str,
) -> Option<Option<String>> {
    let keyword = profile.imports.self_package_root?;
    let rest = specifier.strip_prefix(keyword)?;
    if rest.is_empty() {
        return Some(None);
    }
    rest.strip_prefix(profile.qname_separator)
        .or_else(|| rest.strip_prefix('/'))
        .map(|sub| Some(sub.to_string()))
}

/// `true` when `qualified_name` reads as `module_path` (slash / colon /
/// dot-separated) prefix followed by `.` and one or more segments.
pub(crate) fn qname_under_module(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    qualified_name.starts_with(&needle) || qualified_name == dotted
}

/// Stricter form of `qname_under_module`: candidate must sit DIRECTLY under the
/// module — exactly one segment deeper. `Assertions.assertTrue` matches
/// `Assertions`; `Assertions.Nested.foo` does not.
pub(crate) fn qname_directly_under(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    let Some(rest) = qualified_name.strip_prefix(needle.as_str()) else {
        return false;
    };
    !rest.contains('.')
}

/// Minimum score margin the top candidate must beat the runner-up by for
/// `pick_ranked_candidate` to commit; a smaller margin is too ambiguous to guess.
pub(crate) const RANK_MARGIN: i32 = 100;

/// Score a same-name candidate for import-scoped selection — higher is better.
/// Reads only the candidate row and the use site's file context (its package,
/// imports, path): same workspace package +1000, an import naming the candidate's
/// package +500 or its namespace +300, ambient +200, shared path prefix
/// +10/segment, an external-depth penalty, and a visibility hint.
pub(crate) fn score_candidate(
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    sym: &Symbol,
) -> i32 {
    let mut s: i32 = 0;
    if let (Some(caller_pkg), Some(sym_pkg)) = (file_package_id, sym.package_id) {
        if caller_pkg == sym_pkg {
            s += 1000;
        }
    }
    for import in &file_ctx.imports {
        let Some(mod_path) = import.module_path.as_deref() else {
            continue;
        };
        if let Some(wp_id) = lookup.workspace_package_id(mod_path) {
            if Some(wp_id) == sym.package_id {
                s += 500;
            }
        }
        if qname_under_module(&sym.qualified_name, mod_path) {
            s += 300;
        }
        // A crate-relative qname (Rust, C++) never carries the workspace
        // package's own declared name as a leading qname segment — only a
        // deeper submodule path does. Score the sub-path remainder after
        // peeling the package's declared name off `mod_path`, so a same-
        // package candidate under the submodule the import actually names
        // (`tantivy::schema::Schema`) outranks a same-package candidate at
        // the crate root that merely shares the bare name.
        if let Some(sub) = workspace_sub_path(mod_path, lookup) {
            if qname_under_module(&sym.qualified_name, &sub) {
                s += 300;
            }
        }
    }
    // An implicit/global wildcard namespace (`<Using Include>`, SDK implicit
    // usings, manifest-declared opens) scopes a candidate exactly like a
    // written namespace import: the file sees `Xunit.*` without a `using`
    // line, so `Xunit.Assert` outranks a same-named type from a package no
    // scope names.
    for ns in lookup.implicit_wildcard_namespaces(file_package_id) {
        if qname_under_module(&sym.qualified_name, ns) {
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
        .map(|sym| (score_candidate(file_ctx, file_package_id, lookup, sym), *sym))
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
    let candidate_dir = candidate_norm.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let caller_segs: Vec<&str> = caller_dir.split('/').filter(|s| !s.is_empty()).collect();
    let candidate_segs: Vec<&str> =
        candidate_dir.split('/').filter(|s| !s.is_empty()).collect();
    caller_segs
        .iter()
        .zip(candidate_segs.iter())
        .take_while(|(a, b)| a == b)
        .count() as i32
        * 10
}

/// Strip a leading `{kw}.` from `target` when `kw` is one of `self_keywords`
/// (Python `self.method` → `method`). Only the first matching keyword strips,
/// and only when followed by `.`. An empty `self_keywords` slice returns
/// `target` unchanged.
pub(crate) fn strip_self_keyword<'t>(target: &'t str, self_keywords: &[&str]) -> &'t str {
    for kw in self_keywords {
        if let Some(rest) = target.strip_prefix(kw) {
            if let Some(after) = rest.strip_prefix('.') {
                return after;
            }
        }
    }
    target
}

/// Normalize a name for the bare-name binding comparison. Applied identically to
/// a candidate's name and the ref's target before they are compared.
///
/// `NameNormalization::None` is the identity transform and borrows unchanged. A
/// `Spec` whose deltas are all off is also identity and borrows. Otherwise the
/// transform runs in a fixed order: strip a wrapping sigil pair, strip the first
/// matching leading prefix, remove the configured characters anywhere, then fold
/// ASCII case.
pub(crate) fn normalize_name(norm: NameNormalization, s: &str) -> Cow<'_, str> {
    let spec = match norm {
        NameNormalization::None => return Cow::Borrowed(s),
        NameNormalization::Spec(spec) => spec,
    };
    if is_identity_spec(&spec) {
        return Cow::Borrowed(s);
    }

    let mut cur = s;

    // 1. Sigil wrapper: when the name both starts with `prefix` and ends with
    //    `suffix`, drop both. The first matching pair wins.
    for (prefix, suffix) in spec.strip_sigils {
        if let Some(inner) = cur.strip_prefix(*prefix) {
            if let Some(inner) = inner.strip_suffix(*suffix) {
                cur = inner;
                break;
            }
        }
    }

    // 2. Leading prefix: drop the first declared prefix that matches. A prefix
    //    runs before the case-fold step, so when the spec folds case the prefix
    //    match folds too; a case-sensitive spec keeps the exact byte match.
    for prefix in spec.strip_prefixes {
        let matched_len = if spec.case_insensitive {
            cur.get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| prefix.len())
        } else {
            cur.starts_with(*prefix).then_some(prefix.len())
        };
        if let Some(len) = matched_len {
            cur = &cur[len..];
            break;
        }
    }

    // 3 & 4. Remove the configured characters anywhere and fold case. Both need
    //        an owned buffer; build it once.
    let needs_char_strip = !spec.strip_chars.is_empty();
    if !needs_char_strip && !spec.case_insensitive {
        return Cow::Borrowed(cur);
    }
    let mut out = String::with_capacity(cur.len());
    for ch in cur.chars() {
        if needs_char_strip && spec.strip_chars.contains(&ch) {
            continue;
        }
        if spec.case_insensitive {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

/// A `NormSpec` whose every field is the default (no sigils, no prefixes, no
/// chars, case-sensitive) is the identity transform — `normalize_name` borrows
/// rather than allocating for it.
pub(crate) fn is_identity_spec(spec: &NormSpec) -> bool {
    !spec.case_insensitive
        && spec.strip_chars.is_empty()
        && spec.strip_prefixes.is_empty()
        && spec.strip_sigils.is_empty()
}

#[cfg(test)]
#[path = "support_tests.rs"]
mod tests;
