// =============================================================================
// CST node helpers shared across the PowerShell extractor
//
// Small, generic walks over tree-sitter-powershell nodes — text extraction,
// child lookup by kind, invocation-target inference, recursive command-tag
// resolution. These have no knowledge of symbols or edges and depend only on
// `Node`, the source string, and the cmdlet-type registry.
// =============================================================================

use crate::ecosystem::powershell_cmdlet_types::cmdlet_result_module_tag;
use tree_sitter::Node;

pub(super) fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

pub(super) fn find_child_text(node: &Node, kind: &str, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(&child, src).to_string());
        }
    }
    None
}

/// Like [`find_child_text`] but returns the child node itself, for callers
/// that need to inspect its structure rather than just its source span.
pub(super) fn find_child<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    return node.children(&mut cursor).find(|&child| child.kind() == kind);
}

/// Extract the qualifier (module) from an `invokation_expression` node.
///
/// Patterns handled:
/// - `[Type]::Method()` — first named child is `type_literal`; collect dotted
///   type name from nested `type_identifier` leaves (e.g. `System.IO.File`).
/// - `$obj.Method()`    — first named child is `variable`; strip leading `$`.
/// - `$sync["Key"].Method()` — first named child is `member_access` or
///   `element_access`; walk into the subtree to find the root `variable`.
/// - `(Get-Date).Method()` — first named child is `parenthesized_expression`
///   containing a `command` node; look up the cmdlet in the type table and
///   return the synthetic module tag (matches the sentinel emitted by
///   `emit_dotnet_binding_sentinels`).
pub(super) fn invokation_module(node: &Node, src: &str) -> Option<String> {
    // Find the first named child by index to avoid borrowing cursor across the match.
    let first_named_idx =
        (0..node.child_count()).find(|&i| node.child(i).map_or(false, |c| c.is_named()))?;
    let first = node.child(first_named_idx)?;

    match first.kind() {
        "type_literal" => {
            // Collect all type_identifier leaves in document order, join with "."
            let mut parts: Vec<String> = Vec::new();
            collect_type_identifiers(first, src, &mut parts);
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("."))
            }
        }
        "variable" => {
            let raw = node_text(&first, src);
            let stripped = raw.trim_start_matches('$');
            if stripped.is_empty() {
                None
            } else {
                Some(stripped.to_string())
            }
        }
        // Part 1: `$sync["Key"].Method()` — root variable through element_access chain
        // Part 1: `$sync.Form.FindName(...)` — root variable through member_access chain
        "element_access" | "member_access" => find_root_variable(&first, src),
        // Part 3: `(Get-Date).Method()` — cmdlet result synthetic tag
        "parenthesized_expression" => extract_cmdlet_tag_from_paren(&first, src),
        _ => None,
    }
}

/// Walk down a nested `element_access` / `member_access` / `variable` chain
/// to find the root `variable` node, and return its name (without `$`).
///
/// Handles chains like:
///   `$sync["Key"]`          → element_access { variable($sync), "[", string, "]" }
///   `$sync.Form`            → member_access { variable($sync), ".", member_name }
///   `$sync["Key"].Dispatcher` → member_access { element_access { variable($sync) }, ... }
pub(super) fn find_root_variable(node: &Node, src: &str) -> Option<String> {
    if node.kind() == "variable" {
        let raw = node_text(node, src);
        let stripped = raw.trim_start_matches('$');
        return if stripped.is_empty() {
            None
        } else {
            Some(stripped.to_string())
        };
    }
    // Recurse into the first named child (the object part of the access).
    let first_idx =
        (0..node.child_count()).find(|&i| node.child(i).map_or(false, |c| c.is_named()))?;
    let first = node.child(first_idx)?;
    match first.kind() {
        "variable" => {
            let raw = node_text(&first, src);
            let stripped = raw.trim_start_matches('$');
            if stripped.is_empty() {
                None
            } else {
                Some(stripped.to_string())
            }
        }
        "element_access" | "member_access" => find_root_variable(&first, src),
        _ => None,
    }
}

/// Given a `parenthesized_expression` node, look for a `command` descendant,
/// extract its `command_name`, and return the synthetic module tag if the
/// cmdlet is in the type table.
///
/// The grammar parses `(Get-Date)` as:
///   parenthesized_expression { pipeline { command { command_name: "Get-Date" } } }
/// So we need a recursive descent to reach the command node.
fn extract_cmdlet_tag_from_paren(node: &Node, src: &str) -> Option<String> {
    find_command_tag_recursive(node, src, 0)
}

/// Recursively search for a `command` node under `node` (up to `max_depth`
/// levels deep) and return the cmdlet module tag if found.
fn find_command_tag_recursive(node: &Node, src: &str, depth: usize) -> Option<String> {
    if depth > 4 {
        return None; // guard against pathological nesting
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "command" {
            if let Some(cmd_name) = find_child_text(&child, "command_name", src) {
                if crate::ecosystem::powershell_cmdlet_types::cmdlet_return_type(&cmd_name)
                    .is_some()
                {
                    return Some(cmdlet_result_module_tag(&cmd_name));
                }
            }
        }
        // Recurse into pipeline, statement_list, and other wrappers.
        if let Some(tag) = find_command_tag_recursive(&child, src, depth + 1) {
            return Some(tag);
        }
    }
    None
}

/// Recursively collect all `type_identifier` leaf texts under `node`.
pub(super) fn collect_type_identifiers(node: tree_sitter::Node, src: &str, out: &mut Vec<String>) {
    if node.kind() == "type_identifier" && node.child_count() == 0 {
        let t = node_text(&node, src).to_string();
        if !t.is_empty() {
            out.push(t);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_identifiers(child, src, out);
    }
}

/// First `simple_name` child (used for class/enum names)
pub(super) fn first_simple_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "simple_name" {
            let name = node_text(&child, src).to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}
