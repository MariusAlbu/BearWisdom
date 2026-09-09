//! Object/mapped syntax enters once; member keys and binder uses become IDs.
use super::{
    signatures::{self, SignatureId},
    Capture, TypeExpr, TypeForm,
};
use crate::indexer::lexical::{LexicalBindings, LexicalSyntax, ScopeId};
use crate::type_checker::core::types::{LitValue, MappedModifier, TypeOperator, TypeProperty};
use tree_sitter::Node;

#[derive(Debug)]
pub(crate) struct Forms {
    pub property: &'static str,
    pub index: &'static str,
    pub mapped_clause: &'static str,
    pub identifier: &'static [&'static str],
    pub optional_annotations: &'static [(&'static str, MappedModifier)],
}

fn forms(node: Node, syntax: &LexicalSyntax) -> Option<&'static Forms> {
    syntax
        .type_forms
        .iter()
        .find_map(|(kind, form)| match form {
            TypeForm::Object(forms) if *kind == node.kind() => Some(*forms),
            _ => None,
        })
}

fn mapped<'t>(node: Node<'t>, forms: &Forms) -> Option<(Node<'t>, Node<'t>)> {
    let mut cursor = node.walk();
    let members: Vec<_> = node
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra())
        .collect();
    let [member] = members.as_slice() else {
        return None;
    };
    if member.kind() != forms.index {
        return None;
    }
    let mut cursor = member.walk();
    let clause = member
        .named_children(&mut cursor)
        .find(|n| n.kind() == forms.mapped_clause)?;
    Some((*member, clause))
}

/// The mapped binder is visible in its constraint as well as its remap/value.
/// A self-reference is preserved as a cycle, never rebound to an outer namesake.
pub(in crate::indexer::lexical) fn scope(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
    inherited: ScopeId,
) -> ScopeId {
    let Some((_, clause)) = forms(node, syntax).and_then(|f| mapped(node, f)) else {
        return inherited;
    };
    let Some(name) = clause
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
    else {
        return inherited;
    };
    let scope = graph.add_scope(
        Some(inherited),
        node.start_byte() as u32,
        node.end_byte() as u32,
        false,
    );
    let name = graph.intern(name);
    let binding = graph.declare_type(scope, name);
    let point = node.start_position();
    graph
        .type_parameters
        .insert(binding, (point.row as u32, point.column as u32, 0));
    graph
        .type_parameter_sites
        .insert(binding, SignatureId(super::unique_symbols::span(node)));
    scope
}

pub(super) fn signature(capture: &Capture, node: Node) -> Option<signatures::Signature> {
    let (_, clause) = mapped(node, forms(node, capture.syntax)?)?;
    Some(signatures::Signature {
        id: SignatureId(super::unique_symbols::span(node)),
        declaration: None,
        syntax: Default::default(),
        generics: vec![signatures::Generic {
            name: clause
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(capture.source).ok())
                .and_then(|n| capture.graph.name_id(n)),
            constraint: clause.child_by_field_name("type").map(|n| capture.expr(n)),
            default: None,
        }],
        parameters: vec![],
        result: None,
    })
}

pub(super) fn capture(capture: &Capture, node: Node, forms: &Forms) -> TypeExpr {
    expression(capture, node, forms)
        .map(|op| TypeExpr::Operator(Box::new(op)))
        .unwrap_or(TypeExpr::Unknown)
}

fn expression(capture: &Capture, node: Node, forms: &Forms) -> Option<TypeOperator<TypeExpr>> {
    if node.has_error() || node.is_missing() {
        return None;
    }
    if let Some((member, clause)) = mapped(node, forms) {
        let keys = clause.child_by_field_name("type")?;
        let value = member.child_by_field_name("type")?;
        let optional = forms
            .optional_annotations
            .iter()
            .find(|(kind, _)| *kind == value.kind())?
            .1;
        let mut cursor = member.walk();
        let readonly = if member
            .children(&mut cursor)
            .any(|n| !n.is_named() && n.kind() == "readonly")
        {
            match member.child_by_field_name("sign").map(|n| n.kind()) {
                Some("-") => MappedModifier::Remove,
                None | Some("+") => MappedModifier::Add,
                _ => return None,
            }
        } else {
            MappedModifier::Preserve
        };
        let mut cursor = value.walk();
        let values: Vec<_> = value
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .collect();
        let [value] = values.as_slice() else {
            return None;
        };
        return Some(TypeOperator::Mapped {
            parameter: TypeExpr::SignatureParameter {
                owner: SignatureId(super::unique_symbols::span(node)),
                index: 0,
            },
            keys: capture.expr(keys),
            remap: clause.child_by_field_name("alias").map(|n| capture.expr(n)),
            value: capture.expr(*value),
            optional,
            readonly,
        });
    }
    let mut properties = Vec::new();
    let mut cursor = node.walk();
    for member in node.named_children(&mut cursor).filter(|n| !n.is_extra()) {
        if member.kind() != forms.property && member.kind() != forms.index {
            return None;
        }
        let index = member.kind() == forms.index;
        let key = if index {
            capture.expr(member.child_by_field_name("index_type")?)
        } else {
            let name = member.child_by_field_name("name")?;
            if forms.identifier.contains(&name.kind()) {
                let spelling = name.utf8_text(capture.source).ok()?;
                if spelling.contains('\\') {
                    return None;
                }
                TypeExpr::Literal(LitValue::Str(spelling.into()))
            } else {
                let key = capture.expr(name);
                if !matches!(key, TypeExpr::Literal(_)) {
                    return None;
                }
                key
            }
        };
        let value = capture.expr(member.child_by_field_name("type")?);
        let mut optional = false;
        let mut readonly = false;
        let mut cursor = member.walk();
        for child in member.children(&mut cursor).filter(|n| !n.is_extra()) {
            if child.is_named() {
                if ["name", "type", "index_type"]
                    .iter()
                    .any(|f| member.child_by_field_name(f) == Some(child))
                {
                    continue;
                }
                return None;
            }
            match child.kind() {
                "?" if !index => optional = true,
                "readonly" => readonly = true,
                ":" | "[" | "]" => {}
                _ => return None,
            }
        }
        properties.push(TypeProperty {
            key,
            value,
            optional,
            readonly,
            index,
        });
    }
    Some(TypeOperator::Object(properties))
}

#[cfg(test)]
#[path = "lexical_structural_capture_tests.rs"]
mod tests;
