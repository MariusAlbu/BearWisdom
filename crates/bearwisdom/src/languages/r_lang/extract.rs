// =============================================================================
// languages/r_lang/extract.rs  —  R language extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — binary_operator where op is `<-`/`=` and RHS is function_definition
//   Variable  — binary_operator where op is `<-`/`=` and RHS is not function/class
//   Class     — call where function = "setClass" / "setRefClass" / "R6Class"
//   Method    — call where function = "setMethod" / "setGeneric"
//   Test      — call where function = "test_that" / "it" / "describe"
//
// REFERENCES:
//   Imports   — call where function = "library" / "require" / "requireNamespace"
//   Calls     — namespace_operator (pkg::fn / pkg:::fn) → function name + module=package
//   Calls     — all other call nodes → function name
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

// Class-defining function names
const CLASS_FUNCS: &[&str] = &["setClass", "setRefClass", "R6Class"];
// Method-defining function names
const METHOD_FUNCS: &[&str] = &["setMethod", "setGeneric", "setValidity"];
// Import function names
const IMPORT_FUNCS: &[&str] = &["library", "require", "requireNamespace"];
// Test function names
const TEST_FUNCS: &[&str] = &["test_that", "it", "describe"];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    // NAMESPACE files in R packages use a special directive syntax that the
    // tree-sitter R grammar parses as call expressions, but which we need to
    // treat as symbol *definitions* rather than call references. Short-circuit
    // to the dedicated NAMESPACE parser when the file path ends with NAMESPACE.
    if file_path.ends_with("/NAMESPACE") || file_path.ends_with("\\NAMESPACE") || file_path == "NAMESPACE" {
        return parse_namespace_file(source, file_path);
    }

    let lang: tree_sitter::Language = tree_sitter_r::LANGUAGE.into();

    let mut parser = Parser::new();
    parser.set_language(&lang).expect("Failed to load R grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(root, src, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// NAMESPACE file parser — emits Function symbols for each exported name
// ---------------------------------------------------------------------------

/// Parse an R package NAMESPACE file and emit Function symbols for every
/// exported name. The NAMESPACE format uses R-syntax directives:
///
///   export(func1, func2)          — explicit function exports
///   exportPattern("^[^\\.]")      — regex pattern export (emit as-is)
///   S3method(generic, class)      — S3 method registration (skip — internal)
///   importFrom(pkg, name)         — import from another package (skip)
///   import(pkg)                   — import whole package (skip)
///   useDynLib(...)                — C linkage (skip)
///
/// We emit one `Function` symbol per exported name. For `exportPattern` we
/// emit a single `Function` symbol named `<pattern>` so the resolver has
/// something to match against if needed.
fn parse_namespace_file(source: &str, _file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut line_num: u32 = 0;

    for raw_line in source.lines() {
        let line = raw_line.trim();
        line_num += 1;

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Accumulate multi-line export(...) blocks by detecting unclosed parens.
        // For simplicity we handle the common single-line and the typical
        // multi-line-one-per-arg patterns by splitting on commas after
        // extracting the directive name and its argument list.
        if let Some(rest) = line.strip_prefix("export(") {
            let args = strip_trailing_paren(rest);
            emit_export_args(args, line_num, &mut symbols);
        } else if let Some(rest) = line.strip_prefix("exportPattern(") {
            let pattern = strip_trailing_paren(rest).trim().trim_matches(|c| c == '"' || c == '\'');
            if !pattern.is_empty() {
                symbols.push(make_function_symbol(pattern, line_num));
            }
        } else if let Some(rest) = line.strip_prefix("exportMethods(") {
            // S4 generic method exports — treat same as export()
            let args = strip_trailing_paren(rest);
            emit_export_args(args, line_num, &mut symbols);
        } else if let Some(rest) = line.strip_prefix("exportClasses(") {
            // S4 class exports — emit as Class symbols
            let args = strip_trailing_paren(rest);
            for name in split_namespace_args(args) {
                let clean = name.trim().trim_matches(|c| c == '"' || c == '\'');
                if !clean.is_empty() {
                    symbols.push(make_function_symbol(clean, line_num));
                }
            }
        }
        // S3method, importFrom, import, useDynLib — all skipped intentionally.
    }

    ExtractionResult::new(symbols, vec![], false)
}

/// Extract individual export names from an `export(a, b, c)` argument string
/// (the part after the opening paren, possibly including the closing paren).
fn emit_export_args(args: &str, line_num: u32, symbols: &mut Vec<ExtractedSymbol>) {
    for name in split_namespace_args(args) {
        let clean = name.trim().trim_matches(|c| c == '"' || c == '\'');
        if !clean.is_empty() {
            symbols.push(make_function_symbol(clean, line_num));
        }
    }
}

/// Strip the trailing `)` from a NAMESPACE directive argument string.
/// Handles both `func)` and `func` (already stripped).
fn strip_trailing_paren(s: &str) -> &str {
    s.trim_end_matches(')').trim_end_matches(|c: char| c == ',' || c == ' ')
}

/// Split a comma-separated argument list that may contain quoted strings.
/// Handles backtick-quoted names, single-quoted, and double-quoted exports.
fn split_namespace_args(args: &str) -> Vec<&str> {
    // Simple comma split — good enough for NAMESPACE format which doesn't
    // nest function calls inside argument lists.
    args.split(',')
        .map(|s| s.trim().trim_end_matches(')'))
        .filter(|s| !s.is_empty())
        .collect()
}

fn make_function_symbol(name: &str, line: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: line.saturating_sub(1),
        end_line: line.saturating_sub(1),
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

// ---------------------------------------------------------------------------
// Traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binary_operator" => {
                let idx = extract_binary_operator(&child, src, symbols, refs, parent_index);
                visit(child, src, symbols, refs, idx.or(parent_index));
            }
            "call" => {
                let idx = extract_call(&child, src, symbols, refs, parent_index);
                visit(child, src, symbols, refs, idx.or(parent_index));
            }
            "namespace_operator" => {
                extract_namespace_operator(&child, src, symbols, refs, parent_index);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// binary_operator  →  Function, Variable, or import/class/test via RHS
// ---------------------------------------------------------------------------

fn extract_binary_operator(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let op = get_operator_text(node, src);
    // Assignment operators produce named symbols.
    // Non-assignment operators (arithmetic, logical, comparison, pipe) emit a
    // lightweight Expression symbol so every binary_operator node is covered.
    if !matches!(op.as_str(), "<-" | "=" | "<<-" | "->" | "->>") {
        // Emit an Expression symbol using the LHS (first child) text as name.
        let first_child = node.child(0)?;
        let expr_text = node_text(first_child, src);
        if expr_text.is_empty() {
            return None;
        }
        // Truncate long expressions
        let short = if expr_text.len() > 40 { expr_text[..40].to_string() } else { expr_text };
        let idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: short.clone(),
            qualified_name: short,
            kind: SymbolKind::Variable,
            visibility: None,
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index,
        });
        return Some(idx);
    }

    let (lhs, rhs) = get_lhs_rhs(node, src, &op)?;

    let name = lhs;
    if name.is_empty() {
        return None;
    }

    match rhs.kind() {
        "function_definition" => {
            let params = extract_r_params(&rhs, src);
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name.clone(),
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(format!("{} <- function({})", name, params)),
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
            Some(idx)
        }
        "call" => {
            // Check if the RHS call is a class constructor
            let callee = get_call_function_name(&rhs, src);
            if CLASS_FUNCS.contains(&callee.as_str()) {
                let class_name = get_first_string_arg(&rhs, src).unwrap_or_else(|| name.clone());
                let idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name: class_name.clone(),
                    qualified_name: class_name,
                    kind: SymbolKind::Class,
                    visibility: Some(Visibility::Public),
                    start_line: node.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                    start_col: node.start_position().column as u32,
                    end_col: node.end_position().column as u32,
                    signature: Some(format!("{} <- {}(...)", name, callee)),
                    doc_comment: None,
                    scope_path: None,
                    parent_index,
                });
                // Still emit the Call edge for the R6Class/setClass call itself
                let source_idx = parent_index.unwrap_or(0);
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: callee,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
                return Some(idx);
            }
            // Otherwise emit the call as a ref and fall through to Variable
            extract_call(&rhs, src, symbols, refs, parent_index);
            // Emit variable
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name,
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
            Some(idx)
        }
        _ => {
            // Simple variable assignment
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name,
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
            Some(idx)
        }
    }
}

// ---------------------------------------------------------------------------
// call  →  Method, Test, Imports, or Calls edge
// ---------------------------------------------------------------------------

fn extract_call(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    let callee = get_call_function_name(node, src);
    if callee.is_empty() {
        return None;
    }
    let line = node.start_position().row as u32;

    if IMPORT_FUNCS.contains(&callee.as_str()) {
        if let Some(pkg) = get_first_string_arg(node, src) {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: pkg.clone(),
                kind: EdgeKind::Imports,
                line,
                module: Some(pkg),
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        return None;
    }

    if METHOD_FUNCS.contains(&callee.as_str()) {
        let method_name = get_first_string_arg(node, src).unwrap_or_else(|| callee.clone());
        let idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: method_name.clone(),
            qualified_name: method_name,
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("{}(...)", callee)),
            doc_comment: None,
            scope_path: None,
            parent_index,
        });
        return Some(idx);
    }

    if TEST_FUNCS.contains(&callee.as_str()) {
        let test_name = get_first_string_arg(node, src).unwrap_or_else(|| callee.clone());
        let idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: test_name.clone(),
            qualified_name: test_name,
            kind: SymbolKind::Test,
            visibility: Some(Visibility::Public),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("{}(...)", callee)),
            doc_comment: None,
            scope_path: None,
            parent_index,
        });
        return Some(idx);
    }

    // Generic call → Calls edge
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: callee,
        kind: EdgeKind::Calls,
        line,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
    None
}

// ---------------------------------------------------------------------------
// namespace_operator  →  Calls edge with module qualifier (pkg::fn / pkg:::fn)
// ---------------------------------------------------------------------------

fn extract_namespace_operator(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // lhs is the package name, rhs is the exported/internal function name.
    let pkg = node.child_by_field_name("lhs")
        .map(|n| node_text(n, src))
        .unwrap_or_default();
    let func = node.child_by_field_name("rhs")
        .map(|n| node_text(n, src))
        .unwrap_or_default();
    if pkg.is_empty() || func.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: func,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: Some(pkg),
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_operator_text(node: &Node, src: &[u8]) -> String {
    // The operator is a named child of kind "operator"
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            let text = node_text(child, src);
            if matches!(text.as_str(), "<-" | "=" | "<<-" | "->" | "->>" | "<") {
                return text;
            }
        }
    }
    // Fallback: the operator field
    node.child_by_field_name("operator")
        .map(|n| node_text(n, src))
        .unwrap_or_default()
}

fn get_lhs_rhs<'a>(node: &'a Node, src: &[u8], op: &str) -> Option<(String, Node<'a>)> {
    match op {
        "->" | "->>" => {
            // right-assignment: value -> name
            let rhs_name = node.child_by_field_name("rhs")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            let lhs_node = node.child_by_field_name("lhs")?;
            Some((rhs_name, lhs_node))
        }
        _ => {
            let lhs_name = node.child_by_field_name("lhs")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            // Clean up identifiers (may have backtick quoting)
            let clean = lhs_name.trim_matches('`').to_string();
            let rhs_node = node.child_by_field_name("rhs")?;
            Some((clean, rhs_node))
        }
    }
}

fn get_call_function_name(node: &Node, src: &[u8]) -> String {
    node.child_by_field_name("function")
        .map(|n| node_text(n, src))
        .unwrap_or_default()
}

fn get_first_string_arg(node: &Node, src: &[u8]) -> Option<String> {
    let args = node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        // Look for string nodes directly or argument children containing strings
        if arg.kind() == "string" {
            let raw = node_text(arg, src);
            let stripped = raw.trim_matches(|c| c == '"' || c == '\'');
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
        // argument wrapper
        if arg.kind() == "argument" {
            let mut acursor = arg.walk();
            for achild in arg.children(&mut acursor) {
                if achild.kind() == "string" {
                    let raw = node_text(achild, src);
                    let stripped = raw.trim_matches(|c| c == '"' || c == '\'');
                    if !stripped.is_empty() {
                        return Some(stripped.to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_r_params(func_def: &Node, src: &[u8]) -> String {
    // function_definition has `parameters` field
    let params = match func_def.child_by_field_name("parameters") {
        Some(p) => p,
        None => return String::new(),
    };
    let mut cursor = params.walk();
    let parts: Vec<String> = params
        .children(&mut cursor)
        .filter(|c| c.kind() == "identifier" || c.kind() == "default_parameter")
        .map(|c| node_text(c, src))
        .collect();
    parts.join(", ")
}

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}
