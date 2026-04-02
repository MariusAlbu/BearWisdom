// =============================================================================
// python/helpers.rs  —  Shared utilities for the Python extractor
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

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Python visibility convention:
///   `__name` (dunder without trailing `__`) → Private
///   `_name`                                  → Private
///   everything else                          → Public
pub(super) fn detect_python_visibility(name: &str) -> Option<Visibility> {
    if name.starts_with("__") && !name.ends_with("__") {
        Some(Visibility::Private)
    } else if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    }
}

/// Return the first docstring from a function/class body node.
pub(super) fn extract_docstring(body: &Node, source: &str) -> Option<String> {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            let mut inner = child.walk();
            for expr in child.children(&mut inner) {
                if expr.kind() == "string" {
                    let raw = node_text(&expr, source);
                    let stripped = raw
                        .trim_start_matches("\"\"\"")
                        .trim_end_matches("\"\"\"")
                        .trim_start_matches("'''")
                        .trim_end_matches("'''")
                        .trim_start_matches('"')
                        .trim_end_matches('"')
                        .trim_start_matches('\'')
                        .trim_end_matches('\'')
                        .trim()
                        .to_string();
                    return Some(stripped);
                }
                if expr.kind() == "concatenated_string" {
                    return Some(node_text(&expr, source));
                }
            }
            break;
        }
        if child.kind() != "comment" {
            break;
        }
    }
    None
}

/// Build `def name(params)` or `def name(params) -> return_type`.
pub(super) fn extract_function_signature(node: &Node, source: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let params_node = node.child_by_field_name("parameters")?;
    let name = node_text(&name_node, source);
    let params = node_text(&params_node, source);

    let sig = if let Some(ret) = node.child_by_field_name("return_type") {
        let ret_text = node_text(&ret, source);
        let ret_clean = ret_text.trim_start_matches("->").trim();
        format!("def {name}{params} -> {ret_clean}")
    } else {
        format!("def {name}{params}")
    };

    Some(sig)
}

pub(super) fn is_test_function(name: &str, has_test_decorator: bool) -> bool {
    name.starts_with("test_") || name.starts_with("test") || has_test_decorator
}

/// Extract a simple type name from a Python type annotation node.
pub(super) fn extract_python_type_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "identifier" => node_text(node, source),
        "attribute" => {
            node.child_by_field_name("attribute")
                .map(|a| node_text(&a, source))
                .unwrap_or_default()
        }
        "generic_type" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                return node_text(&name_node, source);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" || child.kind() == "attribute" {
                    return node_text(&child, source);
                }
            }
            String::new()
        }
        "subscript" => {
            node.child_by_field_name("value")
                .map(|v| extract_python_type_name(&v, source))
                .unwrap_or_default()
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(&child, source);
                    if !name.is_empty() && name != "None" {
                        return name;
                    }
                }
            }
            String::new()
        }
    }
}
