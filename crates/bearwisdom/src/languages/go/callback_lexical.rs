//! Go callback syntax and capture policy.
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
    if node.kind() != "func_literal" || node.has_error() {
        return None;
    }
    let parameters = node.child_by_field_name("parameters")?;
    let parameters: Vec<_> = named_children(parameters)
        .into_iter()
        .filter(|parameter| parameter.kind() == "parameter_declaration")
        .flat_map(named_children)
        .filter(|name| name.kind() == "identifier")
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
        annotation: None,
    })
}

fn is_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "method_declaration"
            | "type_declaration"
            | "struct_type"
            | "interface_type"
    )
}

fn barriers(body: Node, source: &[u8]) -> Vec<CallbackBarrier> {
    fn visit(node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        let names = match node.kind() {
            "short_var_declaration" | "assignment_statement" => node
                .child_by_field_name("left")
                .map(named_children)
                .unwrap_or_default(),
            "var_spec" => named_children(node),
            _ => Vec::new(),
        };
        for name in names.into_iter().filter(|name| name.kind() == "identifier") {
            if let Some(name) = parameter(source, name) {
                out.push(CallbackBarrier {
                    name: name.name,
                    range: scoped_range(node, body, |kind| kind == "block"),
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
