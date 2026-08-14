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

use super::contract::util::is_type_like_kind;
use super::contract::SymbolLookup;

/// Rebuild the id-keyed inherits map from the string-keyed one. `evidence`
/// carries `(child_qname, parent_head) → import module` captured when the
/// edge was recorded from the child's file.
pub(super) fn rebuild_inherits_by_id(
    tree: &dyn SymbolLookup,
    inherits: &FxHashMap<String, Vec<String>>,
    evidence: &FxHashMap<(String, String), String>,
) -> FxHashMap<i64, Vec<i64>> {
    let mut out: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
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
            }
        }
    }
    out
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
