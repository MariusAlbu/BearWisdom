//! Java callback syntax and capture policy.

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
    if node.kind() != "lambda_expression" || node.has_error() {
        return None;
    }
    let parameters = node.child_by_field_name("parameters")?;
    let body = node.child_by_field_name("body").or_else(|| {
        named_children(node)
            .into_iter()
            .filter(|child| *child != parameters && child.start_byte() >= parameters.end_byte())
            .last()
    })?;
    Some(CallbackDescriptor {
        body: span(body),
        parameters: parameter_nodes(parameters)
            .into_iter()
            .filter_map(|declaration| parameter(source, declaration))
            .collect(),
        barriers: barriers(body, source),
        boundaries: collect_boundaries(body, is_boundary)
            .into_iter()
            .map(span)
            .collect(),
        outer_capture: OuterCapture::Transparent,
    })
}

fn parameter_nodes(parameters: Node) -> Vec<Node> {
    match parameters.kind() {
        "identifier" => vec![parameters],
        "inferred_parameters" => named_children(parameters)
            .into_iter()
            .filter(|parameter| parameter.kind() == "identifier")
            .collect(),
        "formal_parameters" => named_children(parameters)
            .into_iter()
            .filter(|parameter| parameter.kind() == "formal_parameter")
            .filter_map(|parameter| parameter.child_by_field_name("name"))
            .filter(|name| name.kind() == "identifier")
            .collect(),
        _ => Vec::new(),
    }
}

fn parameter(source: &[u8], declaration: Node) -> Option<CallbackParameter> {
    Some(CallbackParameter {
        declaration: span(declaration),
        name: node_text(source, declaration)?.to_owned(),
        annotation: None,
    })
}

fn barriers(body: Node, source: &[u8]) -> Vec<CallbackBarrier> {
    fn visit(node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        let name = match node.kind() {
            "variable_declarator" => node.child_by_field_name("name"),
            "assignment_expression" => node.child_by_field_name("left"),
            _ => None,
        }
        .filter(|name| name.kind() == "identifier");
        if let Some(name) = name {
            if let Some(name_text) = node_text(source, name) {
                out.push(CallbackBarrier {
                    name: name_text.to_owned(),
                    range: scoped_range(node, body, is_scope),
                });
            }
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
        "method_declaration"
            | "constructor_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
            | "class_body"
            | "anonymous_class_body"
    )
}
