//! Kotlin callback syntax and capture policy.

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
    if node.kind() != "lambda_literal" || node.has_error() {
        return None;
    }
    let parameters = named_children(node)
        .into_iter()
        .find(|child| child.kind() == "lambda_parameters")?;
    let parameters: Vec<_> = named_children(parameters)
        .into_iter()
        .flat_map(parameter_nodes)
        .filter_map(|declaration| parameter(source, declaration))
        .collect();
    if parameters.is_empty() {
        return None;
    }
    Some(CallbackDescriptor {
        body: span(node),
        parameters,
        barriers: barriers(node, source),
        boundaries: collect_boundaries(node, is_boundary)
            .into_iter()
            .map(span)
            .collect(),
        outer_capture: OuterCapture::Transparent,
    })
}

fn parameter_nodes(node: Node) -> Vec<Node> {
    match node.kind() {
        "variable_declaration" => declaration_name(node).into_iter().collect(),
        "multi_variable_declaration" => named_children(node)
            .into_iter()
            .filter(|inner| inner.kind() == "variable_declaration")
            .filter_map(declaration_name)
            .collect(),
        _ => Vec::new(),
    }
}

fn parameter(source: &[u8], declaration: Node) -> Option<CallbackParameter> {
    Some(CallbackParameter {
        declaration: span(declaration),
        name: node_text(source, declaration)?.to_owned(),
    })
}

fn declaration_name(node: Node) -> Option<Node> {
    if node.kind() == "property_declaration" {
        if let Some(pattern) = node.child_by_field_name("name") {
            if matches!(pattern.kind(), "simple_identifier" | "identifier") {
                return Some(pattern);
            }
            if let Some(name) = pattern.child_by_field_name("bound_identifier").or_else(|| {
                named_children(pattern)
                    .into_iter()
                    .find(|child| matches!(child.kind(), "simple_identifier" | "identifier"))
            }) {
                return Some(name);
            }
        }
    }
    if node.kind() == "variable_declaration" {
        return named_children(node)
            .into_iter()
            .find(|child| matches!(child.kind(), "simple_identifier" | "identifier"));
    }
    named_children(node)
        .into_iter()
        .find(|child| child.kind() == "variable_declaration")
        .and_then(declaration_name)
}

fn barriers(body: Node, source: &[u8]) -> Vec<CallbackBarrier> {
    fn add(name: Node, node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        if let Some(name_text) = node_text(source, name) {
            out.push(CallbackBarrier {
                name: name_text.to_owned(),
                range: scoped_range(node, body, is_scope),
            });
        }
    }

    fn visit(node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        match node.kind() {
            "property_declaration" => {
                if let Some(name) = declaration_name(node) {
                    add(name, node, body, source, out);
                }
            }
            "assignment" => {
                if let Some(name) = node
                    .child_by_field_name("left")
                    .filter(|name| name.kind() == "identifier")
                {
                    add(name, node, body, source, out);
                }
            }
            _ => {}
        }
        for child in named_children(node) {
            visit(child, body, source, out);
        }
    }

    let mut out = Vec::new();
    visit(body, body, source, &mut out);
    out
}

fn is_scope(kind: &str) -> bool {
    kind == "block"
}

fn is_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "class_declaration"
            | "object_declaration"
            | "interface_declaration"
            | "enum_class_body"
    )
}
