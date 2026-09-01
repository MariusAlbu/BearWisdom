// =============================================================================
// engine/merge_canonical — declaration-merging canonicalization
//
// Same-qname type declarations that a language's `MergeScope` declares to be
// ONE logical type (TS `interface Foo` + `namespace Foo` in a file, C#
// partial classes in a package) canonicalize onto the set's smallest row id.
// Rows stay physical; the id-keyed indexes merge onto the canonical id and
// the id-keyed reads map any set member to it, so a receiver bound to either
// row walks the union of the set's members — the per-declaration analogue of
// Roslyn folding partial declarations into one symbol.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::TypeId;

use crate::type_checker::profile::language_profile::MergeScope;

use super::contract::{Symbol, TypeInfo};

/// Grouping buckets recorded during ingest: merge-set key → the row ids that
/// share it. Only type-like declarations from languages with a non-`None`
/// merge scope enter.
#[derive(Debug, Default)]
pub(super) struct MergeGroups {
    /// `SameFile` scope: (qualified name, file path) → rows.
    pub(super) by_file: FxHashMap<(String, String), Vec<i64>>,
    /// `SamePackage` scope: (qualified name, package id) → rows.
    pub(super) by_pkg: FxHashMap<(String, Option<i64>), Vec<i64>>,
}

/// The canonical map: every non-canonical member of a 2+-row merge set →
/// the set's smallest id. Deterministic: min-id wins.
pub(super) fn compute(groups: &MergeGroups) -> FxHashMap<i64, i64> {
    let mut out = FxHashMap::default();
    for ids in groups.by_file.values().chain(groups.by_pkg.values()) {
        if ids.len() < 2 {
            continue;
        }
        let canonical = *ids.iter().min().expect("non-empty merge set");
        for &id in ids {
            if id != canonical {
                out.insert(id, canonical);
            }
        }
    }
    out
}

/// Fold the id-keyed indexes onto canonical ids. Idempotent: entries already
/// keyed canonically are untouched, so the pass may re-run after each ingest
/// batch.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply(
    canonical: &FxHashMap<i64, i64>,
    members_by_id: &mut FxHashMap<i64, Vec<i64>>,
    inherits_by_id: &mut FxHashMap<i64, Vec<i64>>,
    inherits_args_by_pair: &mut FxHashMap<(i64, i64), Vec<TypeId>>,
    enclosing_type_by_id: &mut FxHashMap<i64, i64>,
) {
    if canonical.is_empty() {
        return;
    }
    let canon = |id: i64| canonical.get(&id).copied().unwrap_or(id);

    // Member buckets: union each set member's bucket into the canonical one.
    let moved: Vec<i64> = members_by_id
        .keys()
        .copied()
        .filter(|id| canonical.contains_key(id))
        .collect();
    for id in moved {
        let bucket = members_by_id.remove(&id).unwrap_or_default();
        let target = members_by_id.entry(canon(id)).or_default();
        for m in bucket {
            if !target.contains(&m) {
                target.push(m);
            }
        }
    }

    // Supertype edges: re-key children, canonicalize parents, dedup.
    let moved: Vec<i64> = inherits_by_id
        .keys()
        .copied()
        .filter(|id| canonical.contains_key(id))
        .collect();
    for id in moved {
        let parents = inherits_by_id.remove(&id).unwrap_or_default();
        inherits_by_id.entry(canon(id)).or_default().extend(parents);
    }
    for parents in inherits_by_id.values_mut() {
        for p in parents.iter_mut() {
            *p = canon(*p);
        }
        parents.dedup();
    }

    // Edge args: re-key both pair components; first writer wins on collision.
    let moved: Vec<(i64, i64)> = inherits_args_by_pair
        .keys()
        .copied()
        .filter(|&(c, p)| canonical.contains_key(&c) || canonical.contains_key(&p))
        .collect();
    for pair in moved {
        if let Some(args) = inherits_args_by_pair.remove(&pair) {
            inherits_args_by_pair
                .entry((canon(pair.0), canon(pair.1)))
                .or_insert(args);
        }
    }

    // Enclosing types: values point INTO merge sets; keys are member rows
    // (never merged type rows themselves in practice, but map them anyway).
    let moved: Vec<i64> = enclosing_type_by_id
        .keys()
        .copied()
        .filter(|id| canonical.contains_key(id))
        .collect();
    for id in moved {
        if let Some(v) = enclosing_type_by_id.remove(&id) {
            enclosing_type_by_id.entry(canon(id)).or_insert(v);
        }
    }
    for v in enclosing_type_by_id.values_mut() {
        *v = canon(*v);
    }
}

/// Record one ingested symbol into its merge bucket, per the profile's scope.
pub(super) fn record(
    groups: &mut MergeGroups,
    scope: MergeScope,
    sym: &Symbol,
    file_path: &str,
    package_id: Option<i64>,
) {
    if matches!(scope, MergeScope::None) || !merge_eligible_kind(sym) {
        return;
    }
    match scope {
        MergeScope::SameFile => groups
            .by_file
            .entry((sym.qualified_name.clone(), file_path.to_string()))
            .or_default()
            .push(sym.id),
        MergeScope::SamePackage => groups
            .by_pkg
            .entry((sym.qualified_name.clone(), package_id))
            .or_default()
            .push(sym.id),
        MergeScope::None => {}
    }
}

/// Fill each canonical row's type-info slots from its merge siblings — every
/// slot first-writer-wins, so the interface half of a merged declaration
/// supplies generics the namespace half lacks.
pub(super) fn fold_type_info(
    canonical: &FxHashMap<i64, i64>,
    type_info_by_id: &mut FxHashMap<i64, TypeInfo>,
) {
    for (&from, &to) in canonical {
        let Some(src) = type_info_by_id.get(&from).cloned() else {
            continue;
        };
        let dst = type_info_by_id.entry(to).or_default();
        if dst.return_type_id.is_none() {
            dst.return_type_id = src.return_type_id;
        }
        if dst.field_type_id.is_none() {
            dst.field_type_id = src.field_type_id;
        }
        if dst.generic_param_ids.is_empty() {
            dst.generic_param_ids = src.generic_param_ids.clone();
        }
        if dst.generic_param_default_ids.is_empty() {
            dst.generic_param_default_ids = src.generic_param_default_ids.clone();
        }
    }
}

/// Whether `sym`'s kind participates in declaration merging at all: the
/// member-bearing type kinds plus namespace/module (TS merges `interface Foo`
/// with `namespace Foo`). Aliases never merge — an alias row carries
/// expansion semantics, not a member surface.
pub(super) fn merge_eligible_kind(sym: &Symbol) -> bool {
    super::support::is_type_kind(&sym.kind) || matches!(sym.kind.as_str(), "namespace" | "module")
}

#[cfg(test)]
#[path = "merge_canonical_tests.rs"]
mod tests;
