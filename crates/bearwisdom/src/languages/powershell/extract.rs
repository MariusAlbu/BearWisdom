// =============================================================================
// languages/powershell/extract.rs  —  PowerShell symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function   — `function_statement`
//   Class      — `class_statement`
//   Enum       — `enum_statement`
//   Method     — `class_method_definition` (child of class_statement)
//   Property   — `class_property_definition` (child of class_statement)
//
// REFERENCES:
//   Imports    — `using_statement` (using namespace / using module)
//   Calls      — `command` nodes (every cmdlet/function invocation)
//   TypeRef    — type literals in casts, parameter types, property types
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_powershell::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult::empty();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, "");

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    class_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_statement" => {
                let idx = extract_function_indexed(&child, src, symbols, refs, parent_index);
                // Recurse into function body for nested functions/commands
                visit(child, src, symbols, refs, idx.or(parent_index), class_prefix);
            }
            "class_statement" => {
                extract_class(&child, src, symbols, refs, parent_index);
            }
            "enum_statement" => {
                extract_enum(&child, src, symbols, refs, parent_index);
            }
            "using_statement" => {
                extract_using(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "command" => {
                extract_command(&child, src, parent_index.unwrap_or(0), refs);
                // Recurse into command children so that script-block arguments
                // (e.g. `ForEach-Object { $_.Method() }`) are also visited.
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
            "invokation_expression" => {
                let source_idx = parent_index.unwrap_or(0);
                let name = find_child_text(&child, "member_name", src)
                    .or_else(|| find_child_text(&child, "type_name", src))
                    .or_else(|| find_child_text(&child, "simple_name", src))
                    .unwrap_or_else(|| {
                        // Last resort: first named child text
                        (0..child.child_count())
                            .filter_map(|i| child.child(i))
                            .find(|c| c.is_named())
                            .map(|c| node_text(&c, src).to_string())
                            .unwrap_or_default()
                    });
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
        }
    }
}

/// Like extract_function but returns the symbol index for use as parent.
fn extract_function_indexed(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name = match find_child_text(node, "function_name", src) {
        Some(n) => n,
        None => return None,
    };

    let line = node.start_position().row as u32;
    let sig = format!("function {} {{ ... }}", name);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    visit_for_calls(node, src, idx, refs);
    Some(idx)
}

// ---------------------------------------------------------------------------
// Function extraction
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match find_child_text(node, "function_name", src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let sig = format!("function {} {{ ... }}", name);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract calls inside function body
    visit_for_calls(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Class extraction
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match first_simple_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let class_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("class {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract methods and properties inside class body
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_method_definition" => {
                extract_method(&child, src, symbols, refs, class_idx, &name);
            }
            "class_property_definition" => {
                extract_property(&child, src, symbols, refs, class_idx, &name);
            }
            _ => {}
        }
    }
}

fn extract_method(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
    class_name: &str,
) {
    let method_name = match find_child_text(node, "simple_name", src) {
        Some(n) => n,
        None => return,
    };

    let qualified = format!("{}.{}", class_name, method_name);
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: method_name.clone(),
        qualified_name: qualified,
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("{} ({} method)", method_name, class_name)),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });

    visit_for_calls(node, src, idx, refs);
}

fn extract_property(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
    class_name: &str,
) {
    // Property name is in `variable` child — strip leading `$`
    let raw_name = match find_child_text(node, "variable", src) {
        Some(n) => n,
        None => return,
    };
    let prop_name = raw_name.trim_start_matches('$').to_string();
    let qualified = format!("{}.{}", class_name, prop_name);
    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: prop_name.clone(),
        qualified_name: qualified,
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("${}", prop_name)),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });
}

// ---------------------------------------------------------------------------
// Enum extraction
// ---------------------------------------------------------------------------

fn extract_enum(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match first_simple_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("enum {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Using / Import-Module
// ---------------------------------------------------------------------------

fn extract_using(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `using namespace Foo.Bar` or `using module MyModule`
    let text = node_text(node, src);
    let line = node.start_position().row as u32;

    // Extract the module/namespace name from `using module Foo` or `using namespace Foo`
    let target = if let Some(rest) = text.strip_prefix("using module ") {
        rest.trim().to_string()
    } else if let Some(rest) = text.strip_prefix("using namespace ") {
        rest.trim().to_string()
    } else {
        return;
    };

    if !target.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target.clone(),
            kind: EdgeKind::Imports,
            line,
            module: Some(target),
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Command (cmdlet invocation) → Calls edge
// ---------------------------------------------------------------------------

fn extract_command(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The command name is in the `command_name` child
    let cmd_name = match find_child_text(node, "command_name", src) {
        Some(n) => n,
        None => return,
    };

    // For `Import-Module`, try to extract the module name as an Imports edge;
    // fall back to emitting a Calls edge so the command node is always covered.
    if cmd_name.eq_ignore_ascii_case("Import-Module") {
        let mut emitted = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let text = node_text(&child, src);
            if child.kind() == "command_elements" || child.kind() == "string_literal" || child.kind() == "bare_string_literal" {
                let module = text.trim_matches(|c| c == '"' || c == '\'').to_string();
                if !module.is_empty() && module != cmd_name {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: module.clone(),
                        kind: EdgeKind::Imports,
                        line: node.start_position().row as u32,
                        module: Some(module),
                        chain: None,
                    });
                    emitted = true;
                    break;
                }
            }
        }
        if !emitted {
            // Couldn't resolve module name — still emit so the node is covered
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: cmd_name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: cmd_name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Walk subtree collecting command/call nodes
// ---------------------------------------------------------------------------

fn visit_for_calls(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "command" {
            extract_command(&child, src, source_idx, refs);
            // Recurse so script-block args (ForEach-Object { ... }) are visited.
            visit_for_calls(&child, src, source_idx, refs);
        } else if child.kind() == "invokation_expression" {
            // Method call: extract method name with fallbacks
            let name = find_child_text(&child, "member_name", src)
                .or_else(|| find_child_text(&child, "type_name", src))
                .or_else(|| find_child_text(&child, "simple_name", src))
                .unwrap_or_else(|| {
                    (0..child.child_count())
                        .filter_map(|i| child.child(i))
                        .find(|c| c.is_named())
                        .map(|c| node_text(&c, src).to_string())
                        .unwrap_or_default()
                });
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
            visit_for_calls(&child, src, source_idx, refs);
        } else {
            visit_for_calls(&child, src, source_idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

fn find_child_text(node: &Node, kind: &str, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(&child, src).to_string());
        }
    }
    None
}

/// First `simple_name` child (used for class/enum names)
fn first_simple_name(node: &Node, src: &str) -> Option<String> {
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
