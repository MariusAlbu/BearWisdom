//! Profile-attested reference expressions, distinct from raw pointer operators.
use crate::type_checker::core::types::Mutability;
use tree_sitter::Node;

pub(crate) struct BorrowSyntax {
    pub node: &'static str,
    pub operand: &'static str,
    pub mutable: &'static str,
    pub excluded_tokens: &'static [&'static str],
}

impl BorrowSyntax {
    pub(crate) fn capture<'tree>(&self, node: Node<'tree>) -> Option<(Node<'tree>, Mutability)> {
        if node.kind() != self.node || node.has_error() || node.is_missing() {
            return None;
        }
        let mut cursor = node.walk();
        if node
            .children(&mut cursor)
            .any(|child| self.excluded_tokens.contains(&child.kind()))
        {
            return None;
        }
        let mutable = node
            .named_children(&mut cursor)
            .any(|child| child.kind() == self.mutable);
        Some((
            node.child_by_field_name(self.operand)?,
            if mutable {
                Mutability::Mutable
            } else {
                Mutability::Shared
            },
        ))
    }
}

#[cfg(test)]
#[path = "borrow_syntax_tests.rs"]
mod tests;
