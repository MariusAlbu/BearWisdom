//! Grammar data for source-addressed projections; no declaration lookup here.
use tree_sitter::Node;

pub(crate) struct PlaceSyntax {
    pub fields: &'static [(&'static str, &'static str, &'static str)],
    pub named_selectors: &'static [&'static str],
    pub tuple_selectors: &'static [&'static str],
    /// Node kind, operand field (empty = unique named child), operator token.
    pub dereferences: &'static [(&'static str, &'static str, &'static str)],
    pub reference_fields: bool,
}

pub(crate) enum Place<'tree> {
    Field(Node<'tree>, Node<'tree>),
    Dereference(Node<'tree>),
}

impl PlaceSyntax {
    pub(crate) fn recognizes(&self, node: Node) -> bool {
        self.fields.iter().any(|&(kind, _, _)| kind == node.kind())
            || self
                .dereferences
                .iter()
                .any(|&(kind, _, _)| kind == node.kind())
    }

    pub(crate) fn capture<'tree>(&self, node: Node<'tree>) -> Option<Place<'tree>> {
        if node.has_error() || node.is_missing() {
            return None;
        }
        if let Some(&(_, operand, selector)) = self
            .fields
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        {
            return Some(Place::Field(
                node.child_by_field_name(operand)?,
                node.child_by_field_name(selector)?,
            ));
        }
        let &(_, field, operator) = self
            .dereferences
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())?;
        let mut cursor = node.walk();
        let mut tokens = node
            .children(&mut cursor)
            .filter(|n| !n.is_named() && !n.is_extra());
        if tokens.next()?.kind() != operator || tokens.next().is_some() {
            return None;
        }
        drop(tokens);
        let operand = if field.is_empty() {
            let mut children = node.named_children(&mut cursor).filter(|n| !n.is_extra());
            let operand = children.next()?;
            if children.next().is_some() {
                return None;
            }
            operand
        } else {
            node.child_by_field_name(field)?
        };
        Some(Place::Dereference(operand))
    }
}

#[cfg(test)]
#[path = "place_syntax_tests.rs"]
mod tests;
