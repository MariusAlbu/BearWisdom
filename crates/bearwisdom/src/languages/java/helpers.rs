// =============================================================================
// java/helpers.rs  —  Shared utilities for the Java extractor
// =============================================================================

use crate::parser::scope_tree;
use crate::types::Visibility;
use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

/// Detect visibility from the `modifiers` child of a declaration node.
///
/// In tree-sitter-java, `modifiers` is an unnamed child containing unnamed
/// leaf tokens like "public", "private", "protected".
pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mod_text = node_text(child, src);
            // Fast path: scan the modifier text rather than iterating unnamed tokens.
            // This avoids needing child access on unnamed token nodes.
            if mod_text.contains("public") {
                return Some(Visibility::Public);
            }
            if mod_text.contains("private") {
                return Some(Visibility::Private);
            }
            if mod_text.contains("protected") {
                return Some(Visibility::Protected);
            }
            // No visibility keyword → package-private.
            return None;
        }
    }
    None
}

/// Extract a Javadoc comment (`/** ... */`) immediately preceding `node`.
pub(super) fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        let text = node_text(s, src);
        let trimmed = text.trim_start();
        if trimmed.starts_with("/**") {
            return Some(text);
        }
        // Skip plain block comments and whitespace-only siblings.
        if trimmed.starts_with("/*") || trimmed.is_empty() {
            sib = s.prev_sibling();
            continue;
        }
        break;
    }
    None
}

pub(super) fn build_method_signature(node: &Node, src: &[u8]) -> Option<String> {
    let name = node_text(node.child_by_field_name("name")?, src);
    let ret = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| format_params(p, src))
        .unwrap_or_default();
    let sig = format!("{ret} {type_params}{name}{params}").trim().to_string();
    Some(sig)
}

/// Build a compact parameter list string: `(String name, int id)`.
fn format_params(params_node: Node, src: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "formal_parameter" || child.kind() == "spread_parameter" {
            let type_str = child
                .child_by_field_name("type")
                .map(|t| node_text(t, src))
                .unwrap_or_default();
            let name_str = child
                .child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            if type_str.is_empty() {
                parts.push(name_str);
            } else {
                parts.push(format!("{type_str} {name_str}"));
            }
        }
    }
    format!("({})", parts.join(", "))
}

/// Build a qualified name by combining the parent scope path with `name`,
/// then prepending the package if the scope path doesn't already start with it.
pub(super) fn qualify_with_package(
    name: &str,
    parent_scope: Option<&scope_tree::ScopeEntry>,
    package: &str,
) -> String {
    match parent_scope {
        Some(scope) => {
            // Scope already carries the full qualified name up to the parent.
            // If we're in a package and the scope doesn't start with the package,
            // prepend it.
            let base = &scope.qualified_name;
            if !package.is_empty() && !base.starts_with(package) {
                format!("{package}.{base}.{name}")
            } else {
                format!("{base}.{name}")
            }
        }
        None => {
            if package.is_empty() {
                name.to_string()
            } else {
                format!("{package}.{name}")
            }
        }
    }
}

/// Build the scope_path string: the parent's qualified name, prefixed with
/// the package if needed.
pub(super) fn scope_path_with_package(
    parent_scope: Option<&scope_tree::ScopeEntry>,
    package: &str,
) -> Option<String> {
    match parent_scope {
        Some(scope) => {
            let base = &scope.qualified_name;
            if !package.is_empty() && !base.starts_with(package) {
                Some(format!("{package}.{base}"))
            } else {
                Some(base.clone())
            }
        }
        None => {
            if package.is_empty() {
                None
            } else {
                Some(package.to_string())
            }
        }
    }
}

/// Extract the simple (unqualified) name from a type node.
///
/// Handles:
/// - `type_identifier`       → raw text (e.g. "List")
/// - `generic_type`          → first type_identifier child (e.g. "List" from "List<User>")
/// - `scoped_type_identifier` → last segment (e.g. "UserService" from "com.example.UserService")
/// - `array_type`            → recurse into element type
pub(super) fn type_node_simple_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, src),
        "generic_type" => {
            // Children: type_identifier | scoped_type_identifier, type_arguments
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" => return node_text(child, src),
                    "scoped_type_identifier" => {
                        let full = node_text(child, src);
                        return full.rsplit('.').next().unwrap_or(&full).to_string();
                    }
                    _ => {}
                }
            }
            String::new()
        }
        "scoped_type_identifier" => {
            let full = node_text(node, src);
            full.rsplit('.').next().unwrap_or(&full).to_string()
        }
        "array_type" => {
            // element type is the first _unannotated_type child.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let name = type_node_simple_name(child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "annotated_type" => {
            // Strip annotations and recurse into the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "annotation" | "marker_annotation" => continue,
                    _ => {
                        let name = type_node_simple_name(child, src);
                        if !name.is_empty() {
                            return name;
                        }
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Extract a simple type name from a Java type node (for param extraction).
pub(super) fn java_type_node_simple_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, src),
        "generic_type" => {
            // `Repository<User>` — take the outer type name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return node_text(child, src);
                }
            }
            String::new()
        }
        "array_type" => {
            // `User[]` — extract element type.
            node.child_by_field_name("element")
                .map(|e| java_type_node_simple_name(e, src))
                .unwrap_or_default()
        }
        "scoped_type_identifier" => {
            // `java.util.List` — last segment.
            let text = node_text(node, src);
            text.rsplit('.').next().unwrap_or(&text).to_string()
        }
        _ => String::new(),
    }
}

/// Return true for Java primitive types that don't reference user symbols.
pub(super) fn is_java_primitive(name: &str) -> bool {
    matches!(
        name,
        "boolean" | "byte" | "char" | "double" | "float"
            | "int" | "long" | "short" | "void"
            | "String" | "Integer" | "Long" | "Double" | "Float"
            | "Boolean" | "Byte" | "Character" | "Short"
            | "Object" | "Number"
    )
}

pub(super) const TEST_ANNOTATIONS: &[&str] = &[
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "TestFactory",
    "TestTemplate",
];

/// Returns true if any `marker_annotation` or `annotation` in the `modifiers`
/// (or as a direct child of the method node) is a JUnit/TestNG test annotation.
pub(super) fn has_test_annotation(node: &Node, src: &[u8]) -> bool {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        match child.kind() {
            "modifiers" => {
                let mut mc = child.walk();
                for ann in child.children(&mut mc) {
                    if annotation_is_test(ann, src) {
                        return true;
                    }
                }
            }
            "marker_annotation" | "annotation" => {
                if annotation_is_test(child, src) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn annotation_is_test(node: Node, src: &[u8]) -> bool {
    if node.kind() != "marker_annotation" && node.kind() != "annotation" {
        return false;
    }
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(name_node, src);
        return TEST_ANNOTATIONS.contains(&name.as_str());
    }
    false
}

/// Find the parameter name inside a `formal_parameter` node.
pub(super) fn find_formal_param_name(param_node: &Node, src: &[u8]) -> String {
    let mut cursor = param_node.walk();
    for child in param_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                return node_text(child, src);
            }
            _ if child.is_named() => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    return node_text(name_node, src);
                }
            }
            _ => {}
        }
    }
    String::new()
}
