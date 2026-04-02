// =============================================================================
// ruby/helpers.rs  —  Shared utilities for the Ruby extractor
// =============================================================================

use crate::types::Visibility;
use tree_sitter::Node;

pub(super) fn node_text(node: &Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

pub(super) fn ruby_visibility(name: &str) -> Option<Visibility> {
    if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    }
}

/// Extract the bare method name from a `call` node.
///
/// - `foo(...)` — first `identifier` child
/// - `obj.foo(...)` — `method` field
pub(super) fn get_call_method_name(node: &Node, src: &[u8]) -> Option<String> {
    // Named field `method` is set for receiver calls: `obj.method`.
    if let Some(m) = node.child_by_field_name("method") {
        return Some(node_text(&m, src));
    }
    // Bare call: `foo(...)` — walk children for the leading identifier.
    let mut c = node.walk();
    for child in node.children(&mut c) {
        if child.kind() == "identifier" {
            return Some(node_text(&child, src));
        }
    }
    None
}

pub(super) fn build_method_signature(node: &Node, src: &[u8], name: &str) -> Option<String> {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, src))
        .unwrap_or_default();
    Some(format!("def {name}{params}"))
}
