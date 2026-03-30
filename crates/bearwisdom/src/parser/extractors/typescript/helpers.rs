use crate::types::Visibility;
use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // TypeScript accessibility modifiers are direct children.
            "accessibility_modifier" => {
                let text = node_text(child, src);
                match text.as_str() {
                    "public" => return Some(Visibility::Public),
                    "private" => return Some(Visibility::Private),
                    "protected" => return Some(Visibility::Protected),
                    _ => {}
                }
            }
            "export" => return Some(Visibility::Public),
            _ => {}
        }
    }
    None
}

/// Collect a JSDoc comment immediately before `node`.
pub(super) fn extract_jsdoc(node: &Node, src: &[u8]) -> Option<String> {
    let sib = node.prev_sibling()?;
    if sib.kind() == "comment" {
        let text = node_text(sib, src);
        if text.starts_with("/**") {
            return Some(text);
        }
    }
    None
}
