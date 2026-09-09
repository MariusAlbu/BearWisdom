//! Pattern binders use source signatures and branch-local lexical environments.
use super::{
    signatures::{self, SignatureId},
    unique_symbols::span,
    Capture, TypeExpr, TypeForm,
};
use crate::indexer::lexical::{LexicalBindings, LexicalSyntax, ScopeId};
use crate::type_checker::core::types::TypeOperator;
use tree_sitter::Node;

fn form(node: Node, syntax: &LexicalSyntax) -> Option<TypeForm> {
    syntax
        .type_forms
        .iter()
        .find(|(kind, _)| *kind == node.kind())
        .map(|(_, form)| *form)
}

fn name(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let result = node.named_children(&mut cursor).find(|n| !n.is_extra());
    result
}

fn declarations<'t>(node: Node<'t>, syntax: &LexicalSyntax, output: &mut Vec<Node<'t>>) {
    match form(node, syntax) {
        Some(TypeForm::Conditional { .. }) => return,
        Some(TypeForm::Infer) => {
            output.push(node);
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        declarations(child, syntax, output);
    }
}

pub(in crate::indexer::lexical) fn child_scope(
    parent: Node,
    child: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
    inherited: ScopeId,
) -> ScopeId {
    let Some(TypeForm::Conditional {
        extends, when_true, ..
    }) = form(parent, syntax)
    else {
        return inherited;
    };
    if ![extends, when_true]
        .iter()
        .any(|field| parent.child_by_field_name(field) == Some(child))
    {
        return inherited;
    }
    let Some(pattern) = parent.child_by_field_name(extends) else {
        return inherited;
    };
    let mut declarations_ = Vec::new();
    declarations(pattern, syntax, &mut declarations_);
    if declarations_.is_empty() {
        return inherited;
    }
    let scope = graph.add_scope(
        Some(inherited),
        child.start_byte() as u32,
        child.end_byte() as u32,
        false,
    );
    let mut names = std::collections::HashSet::new();
    for declaration in declarations_ {
        let Some(name) = name(declaration).and_then(|n| n.utf8_text(source).ok()) else {
            continue;
        };
        let name = graph.intern(name);
        if !names.insert(name) {
            continue;
        }
        let binding = graph.declare_type(scope, name);
        let point = declaration.start_position();
        graph
            .type_parameters
            .insert(binding, (point.row as u32, point.column as u32, 0));
        graph
            .type_parameter_sites
            .insert(binding, SignatureId(span(declaration)));
    }
    scope
}

pub(super) fn signature(capture: &Capture, node: Node) -> Option<signatures::Signature> {
    if !matches!(form(node, capture.syntax), Some(TypeForm::Infer)) {
        return None;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra())
        .collect();
    let name = children.first()?;
    if children.len() > 2 {
        return None;
    }
    Some(signatures::Signature {
        id: SignatureId(span(node)),
        declaration: None,
        syntax: crate::indexer::lexical::globals::member_surface::Signature {
            type_parameters: vec![span(*name)],
            ..Default::default()
        },
        generics: vec![signatures::Generic {
            name: name
                .utf8_text(capture.source)
                .ok()
                .and_then(|n| capture.graph.name_id(n)),
            constraint: children.get(1).map(|n| capture.expr(*n)),
            default: None,
        }],
        parameters: vec![],
        result: None,
    })
}

pub(super) fn capture(capture: &Capture, node: Node) -> TypeExpr {
    // Only infer declarations collected by a conditional pattern own binders.
    let Some(name) = name(node) else {
        return TypeExpr::Unknown;
    };
    let owner = SignatureId(span(node));
    if !matches!(capture.expr(name), TypeExpr::SignatureParameter { owner: actual, index: 0 } if actual == owner)
    {
        return TypeExpr::Unknown;
    }
    TypeExpr::Operator(Box::new(TypeOperator::Infer(
        TypeExpr::SignatureParameter { owner, index: 0 },
    )))
}

#[cfg(test)]
#[path = "lexical_infer_types_tests.rs"]
mod tests;
