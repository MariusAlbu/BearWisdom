//! Recover exact lexical root nodes at ingestion, never from rewritten display names.
use super::LexicalSyntax;
use tree_sitter::Node;

pub(super) fn reference_root<'tree>(
    root: Node<'tree>,
    byte: u32,
    syntax: &LexicalSyntax,
) -> Option<Node<'tree>> {
    let mut node = root.named_descendant_for_byte_range(byte as usize, byte as usize + 1)?;
    loop {
        if let Some(&(_, field)) = syntax
            .call_roots
            .iter()
            .chain(syntax.reference_roots)
            .find(|&&(kind, _)| kind == node.kind())
        {
            node = node.child_by_field_name(field)?;
        } else if syntax.reference_wrappers.contains(&node.kind()) {
            let mut cursor = node.walk();
            node = node.named_children(&mut cursor).find(|n| !n.is_extra())?;
        } else {
            return Some(node);
        }
    }
}

#[cfg(test)]
#[path = "lexical_reference_roots_tests.rs"]
mod tests;
