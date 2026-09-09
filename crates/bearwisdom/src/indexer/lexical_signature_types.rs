//! Source signature ownership is independent of the navigation row inventory.
use super::{Capture, TypeExpr};
use crate::indexer::lexical::{globals::member_surface, NameId};
use crate::types::SourceSpan;
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct SignatureId(pub(crate) SourceSpan);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemberValue<D = usize> {
    pub owner: D,
    pub name: NameId,
    pub signature: SignatureId,
    pub static_: bool,
    pub readable: bool,
}

pub(super) fn member_value(capture: &Capture, node: Node) -> Option<MemberValue> {
    use member_surface::{Kind, Modifier};
    let forms = capture.syntax.globals.surface;
    let kind = forms
        .kinds
        .iter()
        .find(|&&(kind, _)| kind == node.kind())?
        .1;
    let owner = capture.slot(node.parent()?.parent()?)?;
    let name = node.child_by_field_name("name")?;
    if !capture.syntax.globals.member_names.contains(&name.kind()) {
        return None;
    }
    let mut cursor = node.walk();
    let modifiers: Vec<_> = node
        .children(&mut cursor)
        .filter_map(|n| {
            let token = if !n.is_named() {
                Some(n.kind())
            } else if forms.modifier_wrappers.contains(&n.kind()) {
                n.utf8_text(capture.source).ok()
            } else {
                None
            };
            forms
                .modifiers
                .iter()
                .find(|&&(kind, _)| Some(kind) == token)
                .map(|&(_, modifier)| modifier)
        })
        .collect();
    Some(MemberValue {
        owner,
        name: capture
            .graph
            .name_id(name.utf8_text(capture.source).ok()?)?,
        signature: SignatureId(SourceSpan {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        }),
        static_: modifiers.contains(&Modifier::Static),
        readable: kind == Kind::Property
            && !modifiers.iter().any(|m| {
                matches!(
                    m,
                    Modifier::Optional | Modifier::Private | Modifier::Protected
                )
            }),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Generic<R> {
    pub name: Option<NameId>,
    pub constraint: Option<R>,
    pub default: Option<R>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Signature<R = TypeExpr, D = usize> {
    pub id: SignatureId,
    pub declaration: Option<D>,
    pub syntax: member_surface::Signature,
    pub generics: Vec<Generic<R>>,
    pub parameters: Vec<R>,
    pub result: Option<R>,
}

pub(super) fn capture(capture: &Capture, node: Node) -> Option<Signature> {
    if let Some(signature) = super::inference::signature(capture, node) {
        return Some(signature);
    }
    if let Some(signature) = super::structural::signature(capture, node) {
        return Some(signature);
    }
    let forms = capture.syntax.globals.surface;
    let kind = forms
        .kinds
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
        .map(|&(_, kind)| kind);
    if kind.is_none()
        && node.child_by_field_name("type_parameters").is_none()
        && node.child_by_field_name("parameters").is_none()
        && !(capture
            .syntax
            .callback_forms
            .functions
            .contains(&node.kind())
            && node.child_by_field_name("parameter").is_some())
    {
        return None;
    }
    let syntax =
        member_surface::signature(node, kind.unwrap_or(member_surface::Kind::Unknown), forms);
    let lower = |span: SourceSpan| {
        node.named_descendant_for_byte_range(span.start as usize, span.end as usize)
            .filter(|n| n.start_byte() == span.start as usize && n.end_byte() == span.end as usize)
            .map(|n| capture.expr(n))
            .unwrap_or(TypeExpr::Unknown)
    };
    let generics = node
        .child_by_field_name("type_parameters")
        .map(|parameters| {
            let mut cursor = parameters.walk();
            parameters
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra())
                .map(|p| Generic {
                    name: p
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(capture.source).ok())
                        .and_then(|name| capture.graph.name_id(name)),
                    constraint: p.child_by_field_name("constraint").map(|n| capture.expr(n)),
                    default: p.child_by_field_name("value").map(|n| capture.expr(n)),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Signature {
        id: SignatureId(SourceSpan {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        }),
        declaration: capture.slot(node),
        generics,
        parameters: syntax
            .parameters
            .iter()
            .map(|p| p.type_span.map(lower).unwrap_or(TypeExpr::Unknown))
            .collect(),
        result: syntax.result.map(lower),
        syntax,
    })
}

#[cfg(test)]
#[path = "lexical_signature_types_tests.rs"]
mod tests;
