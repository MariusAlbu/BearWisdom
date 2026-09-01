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

use super::contract::util::is_type_like_kind;
use super::contract::SymbolLookup;

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
pub(super) fn import_evidence_of(
    pf: &crate::types::ParsedFile,
) -> FxHashMap<&str, &str> {
    pf.refs
        .iter()
        .filter(|r| r.is_import_binding || r.kind == crate::types::EdgeKind::Imports)
        .filter_map(|r| r.module.as_deref().map(|m| (r.target_name.as_str(), m)))
        .collect()
}

/// Rebuild the id-keyed inherits map from the string-keyed one. `evidence`
/// carries `(child_qname, parent_head) → import module` captured when the
/// edge was recorded from the child's file.
pub(super) fn rebuild_inherits_by_id(
    tree: &dyn SymbolLookup,
    inherits: &FxHashMap<String, Vec<String>>,
    evidence: &FxHashMap<(String, String), String>,
    arg_ids: &FxHashMap<String, Vec<(String, Vec<TypeId>)>>,
) -> (FxHashMap<i64, Vec<i64>>, FxHashMap<(i64, i64), Vec<TypeId>>) {
    let mut out: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    let mut args_by_pair: FxHashMap<(i64, i64), Vec<TypeId>> = FxHashMap::default();
    for (child_qname, parent_heads) in inherits {
        let Some((child_id, child_pkg)) = tree
            .by_qualified_name(child_qname)
            .map(|c| (c.id, c.package_id))
        else {
            continue;
        };
        for parent_head in parent_heads {
            let ev = evidence
                .get(&(child_qname.clone(), parent_head.clone()))
                .map(String::as_str);
            if let Some(parent_id) =
                resolve_parent_id_scoped(tree, parent_head, child_qname, child_pkg, ev)
            {
                let parents = out.entry(child_id).or_default();
                if !parents.contains(&parent_id) {
                    parents.push(parent_id);
                }
                // The edge's generic args, re-keyed onto the resolved id pair so
                // the id-climb substitution binds the same `extends Base<Arg>`
                // evidence the string climb reads.
                if let Some(edges) = arg_ids.get(child_qname) {
                    if let Some((_, ids)) = edges.iter().find(|(h, _)| h == parent_head) {
                        args_by_pair
                            .entry((child_id, parent_id))
                            .or_insert_with(|| ids.clone());
                    }
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
) -> Option<i64> {
    let simple = parent_head.rsplit('.').next().unwrap_or(parent_head);
    // The child's file imports this exact name from a module: the parent is
    // that module's declaration, never a same-named type elsewhere.
    if let Some(module) = import_module {
        let want = normalize_separators(module);
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
    if let Some(ns) = namespace_of(child_qname) {
        for cand in tree.by_name(simple).iter() {
            if is_type_like_kind(&cand.kind) && qname_under_module(&cand.qualified_name, &ns, simple)
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
    if let Some(parent) = tree.by_qualified_name(parent_head) {
        return Some(parent.id);
    }
    // A supertype that declares nothing cannot satisfy the member lookup the
    // climb exists for. When one name has several type-like declarations —
    // a package that exports both `type X = …` and the `interface X` the
    // members live on — the member-bearing one is the parent the walk needs.
    member_bearing.or(fallback)
}

/// Qualified-name separators vary by language (`\`, `::`, `/`, `.`);
/// canonicalize to `.` so module evidence and qnames compare uniformly.
fn normalize_separators(s: &str) -> String {
    s.replace("::", ".").replace(['\\', '/'], ".")
}

/// The child's enclosing namespace, separator-normalized: everything before
/// the final `.` of the normalized qname. `None` for a top-level name.
fn namespace_of(qname: &str) -> Option<String> {
    let normalized = normalize_separators(qname);
    normalized
        .rfind('.')
        .map(|ix| normalized[..ix].to_string())
        .filter(|ns| !ns.is_empty())
}

/// True when `qname` declares `simple` directly under `module` (both
/// separator-normalized): `PHPUnit\Framework.TestCase` is under
/// `PHPUnit\Framework`.
fn qname_under_module(qname: &str, normalized_module: &str, simple: &str) -> bool {
    normalize_separators(qname) == format!("{normalized_module}.{simple}")
}

#[cfg(test)]
#[path = "parent_resolution_tests.rs"]
mod tests;

/// Attach `extends Base<Arg>` edge args to ladder-resolved (child, parent)
/// pairs: the string store keys args by (child qname, parent HEAD as
/// written); a resolved pair re-keys them by identity when the parent's
/// declared name matches the written head or its simple name.
pub(super) fn attach_edge_args(
    pairs: &[(i64, i64)],
    by_id: &FxHashMap<i64, super::contract::Symbol>,
    arg_ids: &FxHashMap<String, Vec<(String, Vec<TypeId>)>>,
    out: &mut FxHashMap<(i64, i64), Vec<TypeId>>,
) {
    for &(child_id, parent_id) in pairs {
        if out.contains_key(&(child_id, parent_id)) {
            continue;
        }
        let (Some(child), Some(parent)) = (by_id.get(&child_id), by_id.get(&parent_id)) else {
            continue;
        };
        let Some(edges) = arg_ids.get(&child.qualified_name) else {
            continue;
        };
        if let Some((_, ids)) = edges.iter().find(|(head, _)| {
            head == &parent.qualified_name
                || head.rsplit(['.', ':']).next() == Some(parent.name.as_str())
        }) {
            out.insert((child_id, parent_id), ids.clone());
        }
    }
}
