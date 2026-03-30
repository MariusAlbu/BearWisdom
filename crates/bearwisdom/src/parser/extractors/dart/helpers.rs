// =============================================================================
// dart/helpers.rs  —  Shared utilities for the Dart extractor
// =============================================================================

use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

pub(super) fn get_field_text(node: &Node, src: &str, field: &str) -> Option<String> {
    node.child_by_field_name(field).map(|n| node_text(n, src))
}

pub(super) fn first_child_text_of_kind(node: &Node, src: &str, kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(child, src));
        }
    }
    None
}

pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}
