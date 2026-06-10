// =============================================================================
// languages/puppet/refs.rs  —  Puppet resource declarations and ref emitters
// (resource_declaration, include/require, function_call, plus the post-pass
//  sweeps for resource_reference and function_call).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

use super::extract::{make_symbol, node_text};

// ---------------------------------------------------------------------------
// resource_declaration → Variable + Calls to resource type
// ---------------------------------------------------------------------------

pub(super) fn extract_resource_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let res_type = match find_resource_type(node, src) {
        Some(t) => t,
        None => return,
    };
    let title = find_resource_title(node, src).unwrap_or_else(|| res_type.clone());

    let name = format!("{}[{}]", res_type, title);
    let sig = format!("{} {{ '{}': ... }}", res_type, title);

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        parent_index,
    ));

    // Calls edge to the resource type.
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: idx,
        target_name: res_type,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

fn find_resource_type(node: &Node, src: &str) -> Option<String> {
    // resource_declaration has a `type` field or the first class_identifier/identifier child.
    if let Some(t) = node.child_by_field_name("type") {
        return Some(node_text(t, src));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "class_identifier" | "identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn find_resource_title(node: &Node, src: &str) -> Option<String> {
    // The title is typically a string literal after the resource type name.
    if let Some(t) = node.child_by_field_name("title") {
        let raw = node_text(t, src);
        return Some(raw.trim_matches('"').trim_matches('\'').to_string());
    }
    // Fallback: first string child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(child, src);
            return Some(raw.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// include / require → Imports + Calls
// ---------------------------------------------------------------------------

pub(super) fn extract_include_or_require(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);

    // Collect all class identifiers from the statement.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "class_identifier" | "identifier") {
            let name = node_text(child, src);
            // Imports edge.
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: source_idx,
                target_name: name.clone(),
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                col: 0,
                module: Some(name.clone()),
                chain: None,
                byte_offset: child.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
            // Calls edge.
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: source_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: child.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: child.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// function_call → Calls
// ---------------------------------------------------------------------------

pub(super) fn extract_function_call(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    // Emit ref at the function_call node's line (not the identifier child's line)
    // so coverage correlation matches the function_call ref_node_kind.
    let line = node.start_position().row as u32;

    // Try identifier, class_identifier, qualified_name, variable — take the first.
    let name = {
        let mut found = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(
                k,
                "identifier" | "class_identifier" | "variable" | "qualified_name" | "name"
            ) {
                found = node_text(child, src);
                break;
            }
        }
        if found.is_empty() {
            // Fallback: take the first-line text of the node itself (before `(`)
            let raw = node_text(*node, src);
            raw.lines()
                .next()
                .unwrap_or("")
                .split('(')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        } else {
            found
        }
    };

    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: if name.is_empty() {
            "fn".to_string()
        } else {
            name
        },
        kind: EdgeKind::Calls,
        line,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        col: 0,
    });
}

// ---------------------------------------------------------------------------
// Post-pass sweeps
// ---------------------------------------------------------------------------

/// Walk the entire tree and emit a Calls ref for every `function_call` node.
/// This second pass catches function_calls not visited by dispatch_node.
pub(super) fn collect_all_function_calls(node: Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    if node.kind() == "function_call" {
        let line = node.start_position().row as u32;
        let mut name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(
                k,
                "identifier" | "class_identifier" | "variable" | "qualified_name"
            ) {
                name = node_text(child, src);
                break;
            }
        }
        if name.is_empty() {
            let raw = node_text(node, src);
            name = raw
                .lines()
                .next()
                .unwrap_or("")
                .split('(')
                .next()
                .unwrap_or("fn")
                .trim()
                .to_string();
        }
        refs.push(ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: 0,
            target_name: if name.is_empty() {
                "fn".to_string()
            } else {
                name
            },
            kind: EdgeKind::Calls,
            line,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            col: 0,
        });
        // Recurse into function_call children to find nested calls
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_all_function_calls(child, src, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_function_calls(child, src, refs);
    }
}

/// Walk the entire tree and emit a Calls ref for every `resource_reference` node.
pub(super) fn collect_resource_references(node: Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    if node.kind() == "resource_reference" {
        // resource_reference: Type['title'] — always emit at the node's line
        let line = node.start_position().row as u32;
        let mut name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "class_identifier" | "identifier" | "variable") {
                name = node_text(child, src);
                break;
            }
        }
        if name.is_empty() {
            // Take just the type part (before '[')
            let raw = node_text(node, src);
            name = raw.split('[').next().unwrap_or("").trim().to_string();
        }
        refs.push(ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: 0,
            target_name: if name.is_empty() {
                "Resource".to_string()
            } else {
                name
            },
            kind: EdgeKind::TypeRef,
            line,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            col: 0,
        });
        // Still recurse to find nested resource_references
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_resource_references(child, src, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_resource_references(child, src, refs);
    }
}
