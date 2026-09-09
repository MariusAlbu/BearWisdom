//! TS/JS syntax ingestion: exact callback parameter declarations, with no names.
use crate::types::SourceSpan;
use tree_sitter::Node;

pub(super) fn parameters(node: &Node) -> Vec<Option<SourceSpan>> {
    if let Some(parameter) = node.child_by_field_name("parameter") {
        return vec![identifier(parameter)];
    }
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    parameters
        .named_children(&mut cursor)
        .map(identifier)
        .collect()
}

fn identifier(node: Node) -> Option<SourceSpan> {
    let node = match node.kind() {
        "required_parameter" | "optional_parameter" => node.child_by_field_name("pattern")?,
        _ => node,
    };
    (node.kind() == "identifier").then(|| SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    })
}

#[cfg(test)]
#[path = "callback_spans_tests.rs"]
mod tests;
