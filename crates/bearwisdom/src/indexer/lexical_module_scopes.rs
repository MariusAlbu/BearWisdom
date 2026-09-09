//! Source-local module units; syntax and names are consumed only at ingestion.
use super::{Export, ModuleForms};
use crate::indexer::{
    lexical::{LexicalBindings, NameId, ScopeId},
    namespaces::SourceModuleId,
};
use crate::types::SourceSpan;
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Kind {
    #[default]
    Namespace,
    Literal,
    Augmentation,
}

#[derive(Debug, Clone)]
pub(crate) struct Unit {
    pub id: SourceModuleId,
    pub parent: SourceModuleId,
    pub scope: ScopeId,
    pub kind: Kind,
    pub name: Option<NameId>,
    pub range: SourceSpan,
    pub body: SourceSpan,
    pub ambient: bool,
    pub container_valid: bool,
    pub complete: bool,
    pub exports: Vec<Export>,
    pub assignments: Vec<super::ExportTarget>,
    pub stars: Vec<(String, bool)>,
}

pub(in crate::indexer::lexical) fn kind(node: Node, forms: &ModuleForms) -> Option<Kind> {
    let (augmentation, token, _) = forms.augmentation;
    if node.kind() == augmentation && super::token(node, token) {
        return Some(Kind::Augmentation);
    }
    if !forms.containers.contains(&node.kind()) {
        return None;
    }
    Some(
        if node
            .child_by_field_name("name")
            .is_some_and(|n| forms.literal_names.contains(&n.kind()))
        {
            Kind::Literal
        } else {
            Kind::Namespace
        },
    )
}

pub(super) fn capture<'tree>(
    root: Node<'tree>,
    source: &[u8],
    forms: &ModuleForms,
    graph: &mut LexicalBindings,
) -> Vec<(Unit, Node<'tree>)> {
    let mut result: Vec<(Unit, Node<'tree>)> = Vec::new();
    let mut pending = vec![(root, SourceModuleId(0), false)];
    while let Some((node, parent, ambient)) = pending.pop() {
        let ambient = ambient
            || (node.kind() == forms.augmentation.0 && super::token(node, forms.ambient_token));
        let mut owner = parent;
        if let Some(kind) = kind(node, forms) {
            let mut cursor = node.walk();
            let body = node.child_by_field_name("body").or_else(|| {
                node.named_children(&mut cursor)
                    .find(|n| n.kind() == forms.augmentation.2)
            });
            let scope = body.and_then(|body| graph.scope_at(body.start_byte() as u32));
            // Missing bodies remain units with explicit incomplete evidence.
            let scope = scope
                .or_else(|| graph.scope_at(node.start_byte() as u32))
                .unwrap_or(ScopeId(0));
            let name = node
                .child_by_field_name("name")
                .filter(|name| {
                    forms.literal_names.contains(&name.kind())
                        || forms.identifier_names.contains(&name.kind())
                })
                .and_then(|name| super::text(name, source))
                .map(|name| graph.intern(&name));
            owner = SourceModuleId(result.len() as u32 + 1);
            let range = span(node);
            let complete = body.is_some()
                && !node.has_error()
                && (kind == Kind::Augmentation || name.is_some());
            let expected = if parent.0 == 0 {
                Some(ScopeId(0))
            } else {
                result
                    .get(parent.0 as usize - 1)
                    .map(|(unit, _)| unit.scope)
            };
            let outer = graph
                .scope_at(node.start_byte() as u32)
                .and_then(|id| graph.scopes[id.0].parent);
            let container_valid = body.is_some() && outer == expected;
            result.push((
                Unit {
                    id: owner,
                    parent,
                    scope,
                    kind,
                    name,
                    range,
                    body: body.map(span).unwrap_or(range),
                    ambient,
                    container_valid,
                    complete,
                    exports: Vec::new(),
                    assignments: Vec::new(),
                    stars: Vec::new(),
                },
                body.unwrap_or(node),
            ));
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        pending.extend(
            children
                .into_iter()
                .rev()
                .map(|child| (child, owner, ambient)),
        );
    }
    result
}

fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

#[cfg(test)]
#[path = "lexical_module_scopes_tests.rs"]
mod tests;
