// =============================================================================
// engine/parent_resolution — resolve inheritance parent heads to symbol ids
//
// The inherits map records each child's parent HEADS as written (`extends
// TestCase`). Binding a head to a specific declaration ranks the evidence:
// the child file's import of that exact name (its module names the parent's
// home — the one signal that survives homonyms), then a same-package
// candidate, then a member-bearing candidate anywhere. A supertype that
// declares nothing cannot satisfy the member lookup the climb exists for.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::TypeId;
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};

use super::contract::util::is_type_like_kind;
use super::contract::{Symbol, SymbolLookup};
use super::support::{index_qname_parent, join_index_qname};

/// One inheritance edge's source-attested evidence. The child symbol id owns
/// this record, so a qname collision across packages or languages cannot make
/// one edge borrow another file's source profile.
#[derive(Clone)]
pub(super) struct InheritanceEdge {
    pub(super) head: String,
    pub(super) import_module: Option<String>,
    pub(super) profile: &'static LanguageProfile,
    pub(super) arg_ids: Vec<TypeId>,
}

/// Merge ladder-RESOLVED inheritance pairs (child symbol id → parent symbol
/// id) into the climb map. A resolved pair appends if absent — resolution-
/// bound identity joins the name-derived seed, so the walk reaches the parent
/// resolution actually chose.
pub(super) fn apply_resolved(
    map: &mut FxHashMap<i64, Vec<i64>>,
    pairs: impl IntoIterator<Item = (i64, i64)>,
) {
    for (child, parent) in pairs {
        let parents = map.entry(child).or_default();
        if !parents.contains(&parent) {
            parents.push(parent);
        }
    }
}

/// The file's import bindings as `name → module`: `use`-style imports-kind
/// refs and TS import-binding refs alike. The module is the evidence a bare
/// inheritance head resolves through.
pub(super) fn import_evidence_of(pf: &crate::types::ParsedFile) -> FxHashMap<&str, &str> {
    pf.refs
        .iter()
        .filter(|r| r.is_import_binding || r.kind == crate::types::EdgeKind::Imports)
        .filter_map(|r| r.module.as_deref().map(|m| (r.target_name.as_str(), m)))
        .collect()
}

/// Rebuild the id-keyed inherits map from source-attested edges. Each edge is
/// keyed by its child symbol id, while the stored qname remains a canonical
/// lookup key only.
pub(super) fn rebuild_inherits_by_id(
    tree: &dyn SymbolLookup,
    children: &FxHashMap<i64, Symbol>,
    edges_by_child: &FxHashMap<i64, Vec<InheritanceEdge>>,
    legacy_inherits: &FxHashMap<String, Vec<String>>,
) -> (FxHashMap<i64, Vec<i64>>, FxHashMap<(i64, i64), Vec<TypeId>>) {
    let mut out: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    let mut args_by_pair: FxHashMap<(i64, i64), Vec<TypeId>> = FxHashMap::default();
    for (&child_id, edges) in edges_by_child {
        let Some(child) = children.get(&child_id) else {
            continue;
        };
        for edge in edges {
            if let Some(parent_id) = resolve_parent_id_scoped(
                tree,
                &edge.head,
                &child.qualified_name,
                child.package_id,
                edge.import_module.as_deref(),
                edge.profile,
            ) {
                let parents = out.entry(child_id).or_default();
                if !parents.contains(&parent_id) {
                    parents.push(parent_id);
                }
                // Re-key generic arguments onto the resolved identity pair so
                // supertype substitution follows the same edge as the climb.
                if !edge.arg_ids.is_empty() {
                    args_by_pair
                        .entry((child_id, parent_id))
                        .or_insert_with(|| edge.arg_ids.clone());
                }
            }
        }
    }

    // Incremental DB reloads retain resolved, canonical parent qnames but not
    // the source edge profile. Keep those legacy entries as a canonical-dot
    // fallback only when no freshly parsed child with that qname owns source
    // evidence. Fresh identity-owned edges above always take precedence.
    for (child_qname, parent_heads) in legacy_inherits {
        if edges_by_child.keys().any(|child_id| {
            children
                .get(child_id)
                .is_some_and(|child| child.qualified_name == *child_qname)
        }) {
            continue;
        }
        let Some(child) = tree.by_qualified_name(child_qname) else {
            continue;
        };
        for parent_head in parent_heads {
            if let Some(parent_id) = resolve_parent_id_scoped(
                tree,
                parent_head,
                child_qname,
                child.package_id,
                None,
                &DEFAULT_PROFILE,
            ) {
                let parents = out.entry(child.id).or_default();
                if !parents.contains(&parent_id) {
                    parents.push(parent_id);
                }
            }
        }
    }
    (out, args_by_pair)
}

/// Bind one parent head to a declaration id. See the module header for the
/// evidence ranking. `child_qname` supplies the child's own namespace: an
/// unimported bare head names a same-namespace sibling before anything else
/// (`TestCase extends Assert` inside `PHPUnit\Framework` means THAT `Assert`).
pub(super) fn resolve_parent_id_scoped(
    tree: &dyn SymbolLookup,
    parent_head: &str,
    child_qname: &str,
    child_package: Option<i64>,
    import_module: Option<&str>,
    profile: &LanguageProfile,
) -> Option<i64> {
    let simple = profile.simple_name(parent_head);
    // The child's file imports this exact name from a module: the parent is
    // that module's declaration, never a same-named type elsewhere.
    if let Some(module) = import_module {
        let want = profile.index_qname_from_source(module);
        for cand in tree.by_name(simple).iter() {
            if is_type_like_kind(&cand.kind)
                && qname_under_module(&cand.qualified_name, &want, simple)
            {
                return Some(cand.id);
            }
        }
    }
    // No import names the head: the child's own namespace is the next-best
    // evidence — languages resolve unqualified names against the enclosing
    // namespace before any import machinery runs.
    if let Some(ns) = index_qname_parent(child_qname) {
        for cand in tree.by_name(simple).iter() {
            if is_type_like_kind(&cand.kind) && qname_under_module(&cand.qualified_name, ns, simple)
            {
                return Some(cand.id);
            }
        }
    }
    let declares_members = |id: i64| !tree.members_of_id(id).is_empty();
    let mut same_package: Option<i64> = None;
    let mut member_bearing: Option<i64> = None;
    let mut fallback: Option<i64> = None;
    for cand in tree.by_name(simple).iter() {
        if !is_type_like_kind(&cand.kind) {
            continue;
        }
        let in_package = child_package.is_some() && cand.package_id == child_package;
        if in_package && declares_members(cand.id) {
            return Some(cand.id);
        }
        if in_package {
            same_package.get_or_insert(cand.id);
        }
        if declares_members(cand.id) {
            member_bearing.get_or_insert(cand.id);
        }
        fallback.get_or_insert(cand.id);
    }
    if let Some(id) = same_package {
        return Some(id);
    }
    let parent_qname = profile.index_qname_from_source(parent_head);
    if let Some(parent) = tree.by_qualified_name(&parent_qname) {
        return Some(parent.id);
    }
    // A supertype that declares nothing cannot satisfy the member lookup the
    // climb exists for. When one name has several type-like declarations —
    // a package that exports both `type X = …` and the `interface X` the
    // members live on — the member-bearing one is the parent the walk needs.
    member_bearing.or(fallback)
}

/// True when canonical `qname` declares `simple` directly under canonical
/// `module`. Source spelling is normalized through the active profile first.
fn qname_under_module(qname: &str, module: &str, simple: &str) -> bool {
    qname == join_index_qname(module, simple)
}

#[cfg(test)]
#[path = "parent_resolution_tests.rs"]
mod tests;

/// Attach `extends Base<Arg>` edge args to ladder-resolved (child, parent)
/// pairs. Source-attested edges are already keyed by child identity, so a
/// resolved pair cannot draw generic arguments from a colliding declaration.
pub(super) fn attach_edge_args(
    pairs: &[(i64, i64)],
    by_id: &FxHashMap<i64, super::contract::Symbol>,
    edges_by_child: &FxHashMap<i64, Vec<InheritanceEdge>>,
    out: &mut FxHashMap<(i64, i64), Vec<TypeId>>,
) {
    for &(child_id, parent_id) in pairs {
        if out.contains_key(&(child_id, parent_id)) {
            continue;
        }
        let (Some(_child), Some(parent)) = (by_id.get(&child_id), by_id.get(&parent_id)) else {
            continue;
        };
        let Some(edges) = edges_by_child.get(&child_id) else {
            continue;
        };
        if let Some(edge) = edges.iter().find(|edge| {
            edge.head == parent.qualified_name
                || edge.profile.simple_name(&edge.head) == parent.name
                || edge.profile.index_qname_from_source(&edge.head) == parent.qualified_name
        }) {
            if !edge.arg_ids.is_empty() {
                out.insert((child_id, parent_id), edge.arg_ids.clone());
            }
        }
    }
}
