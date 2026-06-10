// =============================================================================
// languages/puppet/definitions.rs  —  Puppet top-level declaration extractors
// (class_definition, defined_resource_type, function_declaration,
//  node_definition).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

use super::extract::{dispatch_node, make_symbol, node_text};

// ---------------------------------------------------------------------------
// class <name> [inherits <parent>] [($params)] { ... }  →  Class
// ---------------------------------------------------------------------------

pub(super) fn extract_class_definition(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_class_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = build_class_signature(node, src, &name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    // Check for `inherits <parent>` — emit Inherits edge.
    if let Some(parent) = find_inherits_name(node, src) {
        refs.push(ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: idx,
            target_name: parent,
            kind: EdgeKind::Inherits,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }

    // Recurse into the class body.
    visit_class_body(node, src, symbols, refs, idx);
}

fn find_class_name(node: &Node, src: &str) -> Option<String> {
    // class_definition: `class` keyword, then class identifier.
    let mut cursor = node.walk();
    let mut saw_class_keyword = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Skip the `class` keyword token.
            "class" => {
                saw_class_keyword = true;
            }
            "identifier" | "class_identifier" if saw_class_keyword => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    // Fallback: first identifier/class_identifier child.
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        if matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn find_inherits_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut in_inherits = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "class_inherits" {
            // class_inherits node contains the parent class identifier.
            let mut cc = child.walk();
            for ic in child.children(&mut cc) {
                if matches!(ic.kind(), "identifier" | "class_identifier") {
                    return Some(node_text(ic, src));
                }
            }
        }
        // Handle inline `inherits` keyword followed by identifier.
        if node_text(child, src) == "inherits" {
            in_inherits = true;
        } else if in_inherits && matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn build_class_signature(node: &Node, src: &str, name: &str) -> String {
    // Take the first line of the class definition as the signature.
    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if first_line.is_empty() {
        format!("class {}", name)
    } else {
        first_line
    }
}

pub(super) fn visit_class_body(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "block" || child.kind() == "body" {
            let mut bc = child.walk();
            for stmt in child.children(&mut bc) {
                dispatch_node(&stmt, src, symbols, refs, Some(parent_index));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// define <name> [($params)] { ... }  →  Class
// ---------------------------------------------------------------------------

pub(super) fn extract_defined_resource_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_define_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let sig = if first_line.is_empty() {
        format!("define {}", name)
    } else {
        first_line
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    visit_class_body(node, src, symbols, refs, idx);
}

fn find_define_name(node: &Node, src: &str) -> Option<String> {
    // defined_resource_type: `define` keyword, then name.
    let mut cursor = node.walk();
    let mut saw_define = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "define" {
            saw_define = true;
        } else if saw_define && matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    // Fallback via named fields.
    if let Some(n) = node.child_by_field_name("class_identifier") {
        return Some(node_text(n, src));
    }
    if let Some(n) = node.child_by_field_name("identifier") {
        return Some(node_text(n, src));
    }
    None
}

// ---------------------------------------------------------------------------
// function <name>(...): <type> { ... }  →  Function
// ---------------------------------------------------------------------------

pub(super) fn extract_function_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_function_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let sig = if first_line.is_empty() {
        format!("function {}", name)
    } else {
        first_line
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));

    // Recurse into function body.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch_node(&child, src, symbols, refs, Some(idx));
    }
}

fn find_function_name(node: &Node, src: &str) -> Option<String> {
    // function_declaration: `function` keyword, then identifier.
    let mut cursor = node.walk();
    let mut saw_function = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "function" {
            saw_function = true;
        } else if saw_function && child.kind() == "identifier" {
            return Some(node_text(child, src));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// node '<name>' { ... }  →  Function
// ---------------------------------------------------------------------------

pub(super) fn extract_node_definition(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_node_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = format!("node '{}'", name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));

    visit_class_body(node, src, symbols, refs, idx);
}

fn find_node_name(node: &Node, src: &str) -> Option<String> {
    // node_definition: `node` keyword, then node_name (string/regex/default/identifier).
    if let Some(nn) = node.child_by_field_name("node_name") {
        return Some(
            node_text(nn, src)
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    // Fallback: first string or identifier after `node`.
    let mut cursor = node.walk();
    let mut saw_node = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "node" {
            saw_node = true;
        } else if saw_node {
            match child.kind() {
                "string" | "identifier" | "default" => {
                    let t = node_text(child, src)
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    return Some(t);
                }
                _ => {}
            }
        }
    }
    None
}
