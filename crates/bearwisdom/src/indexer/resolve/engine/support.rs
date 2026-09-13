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

pub(crate) use super::candidate_rank::pick_ranked_candidate;

pub(crate) use super::path_match::{
    basename_stem_matches, file_path_matches_module, parent_dir, path_contains_segment_run,
    path_stem_matches,
};

pub(crate) use super::reexports::{
    follow_reexports, relative_reexport_candidates, workspace_pkg_barrels,
};

use crate::indexer::resolve::engine::contract::{
    FileContext, Symbol, SymbolInfo, SymbolLookup, TypeInfo, RESOLVED_CONFIDENCE,
};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::type_checker::profile::language_profile::{
    LanguageProfile, NameNormalization, NormSpec,
};
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
    let profile = crate::languages::default_registry().profile_for(&file_ctx.language);
    for import in &file_ctx.imports {
        let names_target = import.bound_name() == name;
        if !names_target {
            continue;
        }
        let Some(spec) = import.module_path.as_deref() else {
            continue;
        };
        let spec = profile.workspace_specifier_path(spec);
        if let Some(pkg) = lookup.workspace_package_id(spec.as_ref()) {
            return Some(pkg);
        }
    }
    None
}

/// The sub-path remainder after the longest declared workspace-package prefix
/// that `specifier` starts with. `None` when `specifier` IS a declared name
/// (no sub-path) or when no workspace package matches.
///
/// Peels only neutral `/` path components. The profile adapts source syntax
/// before this generic package-root lookup runs.
pub(crate) fn workspace_sub_path(
    profile: &LanguageProfile,
    specifier: &str,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    let specifier = profile.workspace_specifier_path(specifier);
    let specifier = specifier.as_ref();
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

/// When `specifier` starts with `profile.imports.self_package_root`, it names
/// this file's own package. Return the sub-path remainder that follows it:
/// `Some(None)` for the bare keyword and `Some(Some(rest))` for a deeper
/// source-qualified path. `None` when the profile carries no such keyword or
/// `specifier` doesn't lead with it, so the caller falls back to the declared-
/// name lookup.
pub(crate) fn self_package_sub_path(
    profile: &LanguageProfile,
    specifier: &str,
) -> Option<Option<String>> {
    profile.self_package_sub_path(specifier)
}

/// `true` when `qualified_name` reads as a source module prefix followed by
/// the index's dotted qname join. The profile owns source qualification; `/`
/// remains the neutral path-segment separator used by module/file evidence.
pub(crate) fn qname_under_module(
    profile: &LanguageProfile,
    qualified_name: &str,
    module_path: &str,
) -> bool {
    let dotted = module_path_to_index_qname(profile, module_path);
    index_qname_under_module(qualified_name, &dotted)
}

fn index_qname_under_module(qualified_name: &str, dotted_module: &str) -> bool {
    let dotted = dotted_module;
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    qualified_name.starts_with(&needle) || qualified_name == dotted
}

/// Whether a candidate row's index key sits at or under the dotted namespace
/// `dotted_namespace`. Index-qname surgery stays in this module: the ranker asks
/// the question, this answers it.
pub(super) fn symbol_under_index_namespace(sym: &Symbol, dotted_namespace: &str) -> bool {
    index_qname_under_module(&sym.qualified_name, dotted_namespace)
}

/// The leaf of a canonical index qname. This intentionally parses the index
/// representation, not source syntax; source names use `LanguageProfile`.
pub(crate) fn index_qname_leaf(qname: &str) -> &str {
    qname.rsplit('.').next().unwrap_or(qname)
}

/// The parent portion of a canonical index qname.
pub(crate) fn index_qname_parent(qname: &str) -> Option<&str> {
    qname.rsplit_once('.').map(|(parent, _)| parent)
}

/// Whether `qname` is equal to or nested under another canonical index key.
pub(crate) fn index_qname_is_or_under(qname: &str, prefix: &str) -> bool {
    qname == prefix
        || qname
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// Whether a canonical index qname is exactly `suffix` or ends in that
/// complete segment path. This parses stored index keys, never source syntax.
pub(crate) fn index_qname_is_or_ends_with(qname: &str, suffix: &str) -> bool {
    qname == suffix
        || qname
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Join canonical index-qname segments. Source-facing rules must first use a
/// profile to convert their source spelling to index keys.
pub(crate) fn join_index_qname(prefix: &str, leaf: &str) -> String {
    if prefix.is_empty() {
        leaf.to_string()
    } else if leaf.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}.{leaf}")
    }
}

/// The stable synthetic index key assigned to an object-literal callable
/// return. `$Ret` is an index convention, not a source-language spelling.
pub(crate) fn index_return_qname(callable_qname: &str) -> String {
    format!("{callable_qname}$Ret")
}

/// Whether a canonical index qname is the synthetic object-return slot.
pub(crate) fn index_qname_is_return_slot(qname: &str) -> bool {
    qname.ends_with("$Ret")
}

/// Stricter form of `qname_under_module`: candidate must sit DIRECTLY under the
/// module — exactly one segment deeper.
pub(crate) fn qname_directly_under(
    profile: &LanguageProfile,
    qualified_name: &str,
    module_path: &str,
) -> bool {
    let dotted = module_path_to_index_qname(profile, module_path);
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    let Some(rest) = qualified_name.strip_prefix(needle.as_str()) else {
        return false;
    };
    !rest.contains('.')
}

/// Convert a source module spelling into the index's dotted qname surface.
/// Profiles declare their source qualifier; the resolver never inventories
/// language separators here.
fn module_path_to_index_qname(profile: &LanguageProfile, module_path: &str) -> String {
    profile.index_qname_from_source(module_path)
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
