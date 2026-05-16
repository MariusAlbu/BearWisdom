// =============================================================================
// languages/groovy/node_helpers.rs  —  Small text/qname helpers over tree-sitter nodes
// =============================================================================

use tree_sitter::Node;

pub(super) fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Get text of a named field child (e.g. `name`, `function`)
pub(super) fn named_field_text(node: &Node, field: &str, src: &str) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(&n, src).to_string())
        .filter(|s| !s.is_empty())
}

/// Build a dotted qualified name from scoped_identifier / identifier children
pub(super) fn build_qualified_name(node: &Node, src: &str) -> String {
    // package_declaration contains a scoped_identifier or identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "identifier" => {
                return node_text(&child, src).to_string();
            }
            _ => {}
        }
    }
    String::new()
}
