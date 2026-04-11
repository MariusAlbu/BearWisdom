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

/// Extract the `type_parameters` clause text from a declaration node and
/// return it as `"<T, U>"`, or the empty string if absent.
///
/// Covers `function f<T>()`, `class C<T>`, `interface I<T, U>`, `type A<T> = ...`,
/// `method<T>(x: T)`, and signature forms. The returned string includes the
/// angle brackets so callers can splice it directly into a signature.
///
/// The tree-sitter TypeScript grammar exposes generics as a child node of kind
/// `type_parameters`. It appears as a direct child on declaration nodes; we
/// find it by scanning children rather than `child_by_field_name` because not
/// every node version exposes it as a named field.
pub(super) fn extract_type_parameters(node: &Node, src: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_parameters" {
            return node_text(child, src);
        }
    }
    String::new()
}
