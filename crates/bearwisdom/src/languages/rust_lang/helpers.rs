// =============================================================================
// rust/helpers.rs  —  Shared utilities for the Rust extractor
// =============================================================================

use crate::types::Visibility;
use tree_sitter::Node;

pub(super) fn node_text(node: &Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()].to_string()
}

pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// Build `scope_path` from the qualified prefix.
pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Build a signature from the first source line of the item, stripping the
/// trailing `{` (and any whitespace before it).
pub(super) fn extract_signature(node: &Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    let first_line = text.lines().next()?;
    let sig = first_line.trim_end_matches('{').trim().to_string();
    if sig.is_empty() {
        None
    } else {
        Some(sig)
    }
}

/// Detect the Rust visibility of an item by inspecting the `visibility_modifier`
/// child node.
///
/// | Source text       | Result                       |
/// |-------------------|------------------------------|
/// | `pub`             | `Some(Visibility::Public)`   |
/// | `pub(crate)`      | `Some(Visibility::Internal)` |
/// | `pub(super)`      | `Some(Visibility::Protected)`|
/// | `pub(in path)`    | `Some(Visibility::Internal)` |
/// | _(no modifier)_   | `Some(Visibility::Private)`  |
pub(super) fn detect_visibility(node: &Node) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let mut has_restriction = false;
            let mut is_super = false;
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "crate" | "self" | "in" => has_restriction = true,
                    "super" => {
                        has_restriction = true;
                        is_super = true;
                    }
                    "identifier" | "scoped_identifier" => has_restriction = true,
                    _ => {}
                }
            }

            return Some(if !has_restriction {
                Visibility::Public
            } else if is_super {
                Visibility::Protected
            } else {
                Visibility::Internal
            });
        }
    }
    // No visibility modifier — Rust default is private
    Some(Visibility::Private)
}

/// Collect consecutive `///` or `//!` doc-comment lines immediately preceding
/// this node (as previous siblings). Also handles `/** ... */` block doc
/// comments. Returns the combined text, or `None` if there are no doc comments.
pub(super) fn extract_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();

    let mut current = node.prev_sibling();
    while let Some(sib) = current {
        match sib.kind() {
            "line_comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("///") || text.starts_with("//!") {
                    lines.push(text);
                    current = sib.prev_sibling();
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = node_text(&sib, source);
                if text.starts_with("/**") {
                    lines.push(text);
                }
                break;
            }
            _ => break,
        }
    }

    if lines.is_empty() {
        return None;
    }

    lines.reverse();
    Some(lines.join("\n"))
}

/// Return `true` if the `function_item` has an `attribute_item` sibling
/// (immediately preceding, possibly separated by other attribute items or
/// comments) that contains `test`.
///
/// Matches `#[test]`, `#[tokio::test]`, `#[async_std::test]`, etc.
pub(super) fn has_test_attribute(node: &Node, source: &str) -> bool {
    let mut current = node.prev_sibling();
    while let Some(sib) = current {
        match sib.kind() {
            "attribute_item" => {
                let text = node_text(&sib, source);
                if text.contains("test") {
                    return true;
                }
                current = sib.prev_sibling();
            }
            "line_comment" | "block_comment" => {
                current = sib.prev_sibling();
            }
            _ => break,
        }
    }
    false
}
