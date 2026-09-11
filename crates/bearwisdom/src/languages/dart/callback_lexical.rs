//! Dart callback syntax and capture policy.
use crate::{
    indexer::callback_lexical::{
        CallbackBarrier, CallbackDescriptor, CallbackLexicalAdapter, CallbackParameter,
        OuterCapture,
    },
    languages::callback_lexical_support::{
        collect_boundaries, named_children, node_text, scoped_range, span,
    },
};
use tree_sitter::Node;

pub(crate) static ADAPTER: CallbackLexicalAdapter = CallbackLexicalAdapter { describe };

fn describe(node: Node, source: &[u8]) -> Option<CallbackDescriptor> {
    if node.kind() != "function_expression" || node.has_error() {
        return None;
    }
    let parameters = named_children(node)
        .into_iter()
        .find(|child| child.kind() == "formal_parameter_list")?;
    let parameters: Vec<_> = named_children(parameters)
        .into_iter()
        .filter(|parameter| parameter.kind() == "formal_parameter")
        .filter_map(|parameter| {
            named_children(parameter)
                .into_iter()
                .find(|child| child.kind() == "identifier")
        })
        .collect();
    if parameters.is_empty() {
        return None;
    }
    let body = node.child_by_field_name("body")?;

    Some(CallbackDescriptor {
        body: span(body),
        parameters: parameters
            .into_iter()
            .filter_map(|node| parameter(source, node))
            .collect(),
        barriers: barriers(body, source),
        boundaries: collect_boundaries(body, is_boundary)
            .into_iter()
            .map(span)
            .collect(),
        outer_capture: OuterCapture::Transparent,
    })
}

fn parameter(source: &[u8], node: Node) -> Option<CallbackParameter> {
    let name = node_text(source, node)?.trim();
    (!name.is_empty() && name != "_").then(|| CallbackParameter {
        declaration: span(node),
        name: name.to_owned(),
    })
}

fn is_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "local_function_declaration"
            | "function_declaration"
            | "class_declaration"
            | "enum_declaration"
            | "extension_declaration"
            | "extension_type_declaration"
            | "mixin_declaration"
    )
}

fn barriers(body: Node, source: &[u8]) -> Vec<CallbackBarrier> {
    fn visit(node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        let name = match node.kind() {
            "assignment_expression" => assignment_name(node),
            "initialized_variable_definition" => node
                .child_by_field_name("name")
                .filter(|name| name.kind() == "identifier"),
            _ => None,
        };
        if let Some(name) = name.and_then(|name| parameter(source, name)) {
            out.push(CallbackBarrier {
                name: name.name,
                range: scoped_range(node, body, |kind| kind == "block"),
            });
        }
        for child in named_children(node) {
            visit(child, body, source, out);
        }
    }

    let mut out = Vec::new();
    visit(body, body, source, &mut out);
    out
}

fn assignment_name(node: Node) -> Option<Node> {
    let left = node.child_by_field_name("left")?;
    if left.kind() == "identifier" {
        return Some(left);
    }
    if left.kind() != "assignable_expression" {
        return None;
    }
    let children = named_children(left);
    (children.len() == 1 && children[0].kind() == "identifier").then_some(children[0])
}
