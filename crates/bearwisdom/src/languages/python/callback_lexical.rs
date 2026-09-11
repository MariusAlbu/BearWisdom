//! Python callback syntax and capture policy.
use crate::{
    indexer::callback_lexical::{
        CallbackBarrier, CallbackDescriptor, CallbackLexicalAdapter, CallbackParameter,
        OuterCapture,
    },
    languages::callback_lexical_support::{collect_boundaries, named_children, node_text, span},
    types::SourceSpan,
};
use tree_sitter::Node;

pub(crate) static ADAPTER: CallbackLexicalAdapter = CallbackLexicalAdapter { describe };

fn describe(node: Node, source: &[u8]) -> Option<CallbackDescriptor> {
    if node.kind() != "lambda" || node.has_error() {
        return None;
    }
    let parameters = node.child_by_field_name("parameters")?;
    let parameters: Vec<_> = named_children(parameters)
        .into_iter()
        .filter(|parameter| parameter.kind() == "identifier")
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
    // `_` is an ordinary Python identifier and is deliberately retained.
    (!name.is_empty()).then(|| CallbackParameter {
        declaration: span(node),
        name: name.to_owned(),
    })
}

fn is_boundary(kind: &str) -> bool {
    matches!(kind, "function_definition" | "class_definition")
}

fn barriers(body: Node, source: &[u8]) -> Vec<CallbackBarrier> {
    fn add(node: Node, range: SourceSpan, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        if let Some(name) = parameter(source, node) {
            out.push(CallbackBarrier {
                name: name.name,
                range,
            });
        }
    }

    fn visit(node: Node, body: Node, source: &[u8], out: &mut Vec<CallbackBarrier>) {
        match node.kind() {
            "assignment" | "augmented_assignment" => {
                if let Some(name) = node
                    .child_by_field_name("left")
                    .filter(|name| name.kind() == "identifier")
                {
                    add(
                        name,
                        SourceSpan {
                            start: node.start_byte() as u32,
                            end: body.end_byte() as u32,
                        },
                        source,
                        out,
                    );
                }
            }
            "named_expression" => {
                if let Some(name) = node
                    .child_by_field_name("name")
                    .filter(|name| name.kind() == "identifier")
                {
                    add(
                        name,
                        SourceSpan {
                            start: node.start_byte() as u32,
                            end: body.end_byte() as u32,
                        },
                        source,
                        out,
                    );
                }
            }
            "for_in_clause" => {
                if let (Some(scope), Some(left)) =
                    (comprehension_scope(node), node.child_by_field_name("left"))
                {
                    for name in pattern_identifiers(left) {
                        add(name, span(scope), source, out);
                    }
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

fn comprehension_scope(node: Node) -> Option<Node> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "list_comprehension"
                | "set_comprehension"
                | "dictionary_comprehension"
                | "generator_expression"
        ) {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

fn pattern_identifiers(node: Node) -> Vec<Node> {
    if node.kind() == "identifier" {
        return vec![node];
    }
    if !matches!(
        node.kind(),
        "tuple_pattern" | "list_pattern" | "pattern_list"
    ) {
        return Vec::new();
    }
    named_children(node)
        .into_iter()
        .flat_map(pattern_identifiers)
        .collect()
}
