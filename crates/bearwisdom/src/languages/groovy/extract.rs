// =============================================================================
// languages/groovy/extract.rs  —  Groovy symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `package_declaration`
//   Class      — `class_declaration`
//   Function   — `function_definition` (top-level `def`)
//   Method     — `method_declaration` (inside class body)
//   Variable   — `declaration` (module-level)
//
// REFERENCES:
//   Imports    — `import_declaration`
//   Calls      — `method_invocation`
//
// Grammar: tree-sitter-groovy.  Actual node kinds confirmed by CST probe:
//   class_declaration  (fields: name, body)
//   method_declaration (fields: type, name, parameters, body)
//   function_definition (fields: name, parameters, body)   ← top-level `def fn`
//   package_declaration
//   import_declaration
//   method_invocation  (fields: name, arguments)
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
            "package_declaration" => {
                extract_package(&child, src, symbols, parent_index);
            }
            "class_declaration" => {
                extract_class(&child, src, symbols, refs, parent_index);
            }
            // Top-level `def fn(...)` — grammar emits function_definition
            "function_definition" => {
                extract_function(&child, src, symbols, refs, parent_index, inside_class);
            }
            // Typed `ReturnType method(...)` inside a class — grammar emits method_declaration
            "method_declaration" => {
                extract_method_declaration(&child, src, symbols, refs, parent_index);
            }
            "import_declaration" => {
                extract_import(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "method_invocation" => {
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
    // class_declaration has a `name` field (identifier)
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
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method_declaration" => {
                    extract_method_declaration(&child, src, symbols, refs, Some(class_idx));
                }
                "function_definition" => {
                    extract_function(&child, src, symbols, refs, Some(class_idx), true);
                }
                "method_invocation" => {
                    extract_call(&child, src, class_idx, refs);
                }
                _ => {
                    visit_for_calls(&child, src, class_idx, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function (top-level `def fn(...)`)
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
) {
    // function_definition has a `name` field
    let name = match named_field_text(node, "name", src) {
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
        signature: Some(format!("def {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    visit_for_calls(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Method (typed form: `int add(int a, int b)`)
// ---------------------------------------------------------------------------

fn extract_method_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // method_declaration has fields: type, name, parameters, body
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let return_type = named_field_text(node, "type", src).unwrap_or_default();
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    let sig = if return_type.is_empty() {
        name.clone()
    } else {
        format!("{} {}", return_type, name)
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Method,
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
// Call extraction (method_invocation)
// ---------------------------------------------------------------------------

fn extract_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // method_invocation has field `name` (identifier)
    let name = match named_field_text(node, "name", src) {
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
// Walk subtree collecting method_invocation nodes
// ---------------------------------------------------------------------------

fn visit_for_calls(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_invocation" => {
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

/// Build a dotted qualified name from scoped_identifier / identifier children
fn build_qualified_name(node: &Node, src: &str) -> String {
    // package_declaration contains a scoped_identifier or identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "identifier" => {
                return node_text(&child, src).to_string();
            }
            _ => {}
        }
    }
    String::new()
}
