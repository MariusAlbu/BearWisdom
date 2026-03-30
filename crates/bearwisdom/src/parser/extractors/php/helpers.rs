// =============================================================================
// php/helpers.rs  —  Shared utilities for the PHP extractor
// =============================================================================

use crate::types::{SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn node_text(node: &Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

/// Dot-separated qualifier (used for qualified names within a namespace).
pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// Backslash-separated qualifier for namespace symbols themselves.
pub(super) fn qualify_ns(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}

/// Read the visibility modifier of a method or property declaration.
/// Defaults to Public if no modifier is present (interfaces, enum methods, etc.).
pub(super) fn extract_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(&child, src);
            return match text.as_str() {
                "public" => Some(Visibility::Public),
                "protected" => Some(Visibility::Protected),
                "private" => Some(Visibility::Private),
                _ => Some(Visibility::Public),
            };
        }
    }
    Some(Visibility::Public)
}

pub(super) fn build_method_signature(node: &Node, src: &[u8], name: &str) -> Option<String> {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(": {}", node_text(&r, src)))
        .unwrap_or_default();
    Some(format!("function {name}{params}{ret}"))
}

pub(super) fn build_class_signature(
    node: &Node,
    src: &[u8],
    name: &str,
    kind: SymbolKind,
) -> Option<String> {
    let keyword = match kind {
        SymbolKind::Interface => "interface",
        _ => "class",
    };

    let base = node
        .child_by_field_name("base_clause")
        .map(|b| format!(" extends {}", node_text(&b, src).trim_start_matches("extends ").trim()))
        .unwrap_or_default();

    let impls = node
        .child_by_field_name("class_implements")
        .map(|i| format!(" implements {}", node_text(&i, src).trim_start_matches("implements ").trim()))
        .unwrap_or_default();

    Some(format!("{keyword} {name}{base}{impls}"))
}
