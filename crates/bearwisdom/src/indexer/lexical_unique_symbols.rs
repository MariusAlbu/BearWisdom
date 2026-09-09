//! Profile-owned unique annotation forms. A valid source origin is captured
//! without requiring an extractor's physical navigation row.
use crate::types::SourceSpan;
use tree_sitter::Node;

#[derive(Debug)]
pub(crate) struct Forms {
    pub tokens: &'static [&'static str],
    pub annotation: &'static str,
    pub transparent: &'static [&'static str],
    pub properties: &'static [(&'static str, &'static [&'static str])],
    pub variable: (&'static str, &'static str, &'static str, &'static str),
    pub names: &'static [&'static str],
}

/// None: not this syntax. Some(None): recognized but invalid/unsupported
/// declaration owner. An invalid unique annotation must not become a name.
pub(super) fn owner<'a>(node: Node<'a>, forms: &Forms) -> Option<Option<Node<'a>>> {
    let mut cursor = node.walk();
    let tokens: Vec<_> = node
        .children(&mut cursor)
        .filter(|n| !n.is_extra())
        .map(|n| n.kind())
        .collect();
    if tokens != forms.tokens {
        return None;
    }
    Some((|| {
        let mut parent = node.parent()?;
        while forms.transparent.contains(&parent.kind()) {
            parent = parent.parent()?;
        }
        let annotation = (parent.kind() == forms.annotation).then_some(parent)?;
        let owner = annotation.parent()?;
        let name = owner.child_by_field_name("name")?;
        if !forms.names.contains(&name.kind()) {
            return None;
        }
        if let Some((_, required)) = forms
            .properties
            .iter()
            .find(|(kind, _)| *kind == owner.kind())
        {
            let mut cursor = owner.walk();
            let tokens: Vec<_> = owner
                .children(&mut cursor)
                .filter(|n| !n.is_named())
                .map(|n| n.kind())
                .collect();
            return required
                .iter()
                .all(|token| tokens.contains(token))
                .then_some(owner);
        }
        let (declarator, declaration, field, token) = forms.variable;
        if owner.kind() != declarator {
            return None;
        }
        let parent = owner.parent().filter(|n| n.kind() == declaration)?;
        (parent.child_by_field_name(field)?.kind() == token).then_some(owner)
    })())
}

pub(super) fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

#[cfg(test)]
#[path = "lexical_unique_symbols_tests.rs"]
mod tests;
