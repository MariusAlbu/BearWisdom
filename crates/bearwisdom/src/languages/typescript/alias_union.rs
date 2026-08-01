// =============================================================================
// languages/typescript/alias_union — a union's branch list
//
// The grammar parses `A | B | C` as a LEFT-NESTED spine —
// `union_type(union_type(A, B), C)` — so a single pass over the outer node's
// children sees one nested `union_type` plus the final arm, and every branch
// but the last is lost. A union of string literals is the key set of any
// mapped type built over it, so losing arms leaves such a mapped type with
// almost no keys.
// =============================================================================

use tree_sitter::Node;

use super::alias_type_text::branch_type_text;

/// Every branch name of `node` (a `union_type`), flattening the nested spine.
/// The second element is `true` when any arm is an anonymous object type —
/// those name nothing, so a union of only object arms yields no branches and
/// the caller classifies it structurally instead.
pub(super) fn union_branches(node: &Node, src: &[u8]) -> (Vec<String>, bool) {
    let mut branches = Vec::new();
    let mut has_object_branch = false;
    collect(node, src, &mut branches, &mut has_object_branch);
    (branches, has_object_branch)
}

fn collect(node: &Node, src: &[u8], branches: &mut Vec<String>, has_object_branch: &mut bool) {
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        if child.kind() == "|" {
            continue;
        }
        if child.kind() == "union_type" {
            collect(&child, src, branches, has_object_branch);
            continue;
        }
        if child.kind() == "object_type" {
            *has_object_branch = true;
        }
        let name = branch_type_text(&child, src);
        if !name.is_empty() {
            branches.push(name);
        }
    }
}

#[cfg(test)]
#[path = "alias_union_tests.rs"]
mod tests;
