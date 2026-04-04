// =============================================================================
// languages/groovy/extract.rs  —  Groovy symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `groovy_package`
//   Class      — `class_definition`
//   Function   — `function_definition` (top-level)
//   Method     — `function_definition` (inside class)
//   Variable   — `declaration` (module-level)
//
// REFERENCES:
//   Imports    — `groovy_import`
//   Calls      — `function_call`, `juxt_function_call`
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
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

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, false);

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
    inside_class: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "groovy_package" => {
                extract_package(&child, src, symbols, parent_index);
            }
            "class_definition" => {
                extract_class(&child, src, symbols, refs, parent_index);
            }
            "function_definition" | "function_declaration" => {
                extract_function(&child, src, symbols, refs, parent_index, inside_class);
            }
            "groovy_import" => {
                extract_import(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "function_call" | "juxt_function_call" => {
                extract_call(&child, src, parent_index.unwrap_or(0), refs);
                visit(child, src, symbols, refs, parent_index, inside_class);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index, inside_class);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Package / Namespace
// ---------------------------------------------------------------------------

fn extract_package(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // groovy_package contains a qualified_name
    let name = build_qualified_name(node, src);
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("package {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
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
    let name = match named_field_text(node, "name", src) {
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

    // Walk class body for methods
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" | "function_declaration" => {
                extract_function(&child, src, symbols, refs, Some(class_idx), true);
            }
            "function_call" | "juxt_function_call" => {
                extract_call(&child, src, class_idx, refs);
            }
            _ => {
                // Recurse into other class body nodes for calls
                visit_for_calls(&child, src, class_idx, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function / Method extraction
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
) {
    let name = match named_field_text(node, "function", src)
        .or_else(|| named_field_text(node, "name", src))
    {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let kind = if inside_class { SymbolKind::Method } else { SymbolKind::Function };
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("{}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Collect calls inside function body
    visit_for_calls(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn extract_import(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    // Strip `import ` prefix and any `as Alias` suffix
    let module = text
        .trim_start_matches("import")
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('*')
        .trim_end_matches('.')
        .to_string();

    if module.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: module.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(module),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match named_field_text(node, "function", src) {
        Some(n) => n,
        None => return,
    };

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Walk subtree collecting call nodes
// ---------------------------------------------------------------------------

fn visit_for_calls(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_call" | "juxt_function_call" => {
                extract_call(&child, src, source_idx, refs);
                visit_for_calls(&child, src, source_idx, refs);
            }
            _ => {
                visit_for_calls(&child, src, source_idx, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Get text of a named field child (e.g. `name`, `function`)
fn named_field_text(node: &Node, field: &str, src: &str) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(&n, src).to_string())
        .filter(|s| !s.is_empty())
}

/// Build a dotted qualified name from identifier children
fn build_qualified_name(node: &Node, src: &str) -> String {
    let mut parts = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "qualified_name" {
            let text = node_text(&child, src);
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }
    parts.join(".")
}
