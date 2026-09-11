//! Swift callback syntax and capture policy.
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
    let ty = node.child_by_field_name("type")?;
    let parameters = named_children(ty)
        .into_iter()
        .find(|child| child.kind() == "lambda_function_type_parameters")?;
    let parameters: Vec<_> = named_children(parameters)
        .into_iter()
        .filter(|parameter| parameter.kind() == "lambda_parameter")
        .filter_map(|parameter| {
            parameter
                .child_by_field_name("name")
                .filter(|name| name.kind() == "simple_identifier")
        })
        .collect();
    if parameters.is_empty() {
        // Shorthand `$0` / `$1` parameters have no declaration span.
        return None;
    }

    Some(CallbackDescriptor {
        body: span(node),
        parameters: parameters
            .into_iter()
            .filter_map(|node| parameter(source, node))
            .collect(),
        barriers: barriers(node, source),
        boundaries: collect_boundaries(node, is_boundary)
            .into_iter()
            .map(span)
            .collect(),
        outer_capture: OuterCapture::Transparent,
    })
}

fn parameter(source: &[u8], node: Node) -> Option<CallbackParameter> {
    let name = node_text(source, node)?.trim();
    (!name.is_empty()).then(|| CallbackParameter {
        declaration: span(node),
        name: name.to_owned(),
        annotation: None,
    })
}

fn is_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "init_declaration"
            | "deinit_declaration"
            | "class_declaration"
            | "struct_declaration"
            | "protocol_declaration"
            | "enum_declaration"
    )
}

fn barriers(body: Node, source: &[u8]) -> Vec<CallbackBarrier> {
    fn visit(node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        let name = match node.kind() {
            "property_declaration" | "variable_declaration" => declaration_name(node),
            "assignment" => assignment_name(node),
            _ => None,
        };
        if let Some(name) = name.and_then(|name| parameter(source, name)) {
            out.push(CallbackBarrier {
                name: name.name,
                range: scoped_range(node, body, |kind| matches!(kind, "block" | "statements")),
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

fn declaration_name(node: Node) -> Option<Node> {
    if node.kind() == "property_declaration" {
        let name = node.child_by_field_name("name")?;
        if name.kind() == "simple_identifier" {
            return Some(name);
        }
        return name.child_by_field_name("bound_identifier").or_else(|| {
            named_children(name)
                .into_iter()
                .find(|child| child.kind() == "simple_identifier")
        });
    }
    if node.kind() == "variable_declaration" {
        return named_children(node)
            .into_iter()
            .find(|child| child.kind() == "simple_identifier");
    }
    None
}

fn assignment_name(node: Node) -> Option<Node> {
    let target = node.child_by_field_name("target")?;
    if target.kind() == "simple_identifier" {
        return Some(target);
    }
    named_children(target)
        .into_iter()
        .find(|child| child.kind() == "simple_identifier")
}
