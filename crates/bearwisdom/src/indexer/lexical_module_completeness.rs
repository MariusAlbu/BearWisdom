// =============================================================================
// indexer/lexical_module_completeness — whether a parse read the whole module
// surface
//
// The module surface is the set of names a file declares, imports, exports
// and forwards. A tree the grammar could not fully parse is complete for
// that purpose when every error sits inside a declaration's own body or
// signature: the body's meaning is uncertain, the declaration's name and
// its place in the export table are not. An error anywhere else — in an
// import or export clause, between top-level statements, in a declaration
// header — may have swallowed a name, so nothing can be concluded about
// absence from such a module.
// =============================================================================

use tree_sitter::Node;

use super::modules::ModuleForms;

/// `true` when no parse error reaches the module surface: the tree is
/// error-free, or every error and missing node lies inside one of the
/// forms' `error_containers`.
pub(crate) fn surface_complete(root: Node, forms: &ModuleForms) -> bool {
    if !root.has_error() {
        return true;
    }
    if forms.error_containers.is_empty() {
        return false;
    }
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if !node.has_error() {
            continue;
        }
        if (node.is_error() || node.is_missing()) && !contained(node, forms) {
            return false;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    true
}

/// Whether some ancestor of `node` is a declaration body or signature.
fn contained(node: Node, forms: &ModuleForms) -> bool {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if forms.error_containers.contains(&ancestor.kind()) {
            return true;
        }
        current = ancestor.parent();
    }
    false
}

#[cfg(test)]
#[path = "lexical_module_completeness_tests.rs"]
mod tests;
