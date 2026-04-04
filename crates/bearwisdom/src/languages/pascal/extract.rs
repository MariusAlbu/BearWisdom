// =============================================================================
// languages/pascal/extract.rs  —  Pascal / Delphi extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — declProc / defProc (procedure_declaration / function_declaration)
//   Class     — declClass (class_type)
//   Interface — declIntf (interface_type)
//   Struct    — declSection with record keyword (record_type)
//   Namespace — unit (unit declaration)
//
// REFERENCES:
//   Imports   — declUses (uses clause)
//   Calls     — exprCall (function/method calls)
//   TypeRef   — typeref nodes (type references in signatures)
//
// Grammar: tree-sitter-pascal 0.10.2 (tree-sitter-language ABI, LANGUAGE constant).
// Pascal uses '.' as namespace separator in unit names.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_pascal::LANGUAGE.into())
        .expect("Failed to load Pascal grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_root(tree.root_node(), source, &mut symbols, &mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Root traversal
// ---------------------------------------------------------------------------

fn visit_root(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, None);
    }
}

fn dispatch(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "unit" => extract_unit(node, src, symbols, refs),
        "program" | "library" => extract_program(node, src, symbols, refs),
        "declProc" | "defProc" => extract_proc(node, src, symbols, refs, parent_index),
        "declClass" => extract_class(node, src, symbols, refs, parent_index),
        "declIntf" => extract_intf(node, src, symbols, refs, parent_index),
        "declSection" => extract_section(node, src, symbols, refs, parent_index),
        "declUses" => extract_uses(node, src, refs, parent_index),
        "exprCall" => extract_call(node, src, refs, parent_index),
        _ => {
            // Recurse into containers.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// unit <Name>;  →  Namespace
// ---------------------------------------------------------------------------

fn extract_unit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "unit".to_string());
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Namespace,
        &node,
        None,
        None,
    ));

    // Recurse into unit body.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

fn extract_program(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "program".to_string());
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Namespace,
        &node,
        None,
        None,
    ));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// procedure/function declarations  →  Function
// declProc = forward declaration header only
// defProc  = full definition with body
// ---------------------------------------------------------------------------

fn extract_proc(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_proc_name(node, src)
        .unwrap_or_else(|| "unknown".to_string());

    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        &node,
        Some(sig),
        parent_index,
    ));

    // Recurse into body for nested procs and calls.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

fn find_proc_name(node: Node, src: &str) -> Option<String> {
    // Pascal proc names: first identifier/operatorName child after kFunction/kProcedure
    let mut cursor = node.walk();
    let mut saw_keyword = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "kFunction" | "kProcedure" | "kConstructor" | "kDestructor" | "kOperator" => {
                saw_keyword = true;
            }
            "identifier" | "operatorName" if saw_keyword => {
                return Some(node_text(child, src));
            }
            // Qualified name: TypeName.MethodName
            "genericDot" | "exprDot" if saw_keyword => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    // Fallback: first identifier child.
    find_identifier_child(node, src)
}

// ---------------------------------------------------------------------------
// class type declarations  →  Class
// ---------------------------------------------------------------------------

fn extract_class(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_decl_type_name(node, src)
        .unwrap_or_else(|| "unknown".to_string());

    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        &node,
        Some(sig),
        parent_index,
    ));

    // Recurse for nested members.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// interface type declarations  →  Interface
// ---------------------------------------------------------------------------

fn extract_intf(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_decl_type_name(node, src)
        .unwrap_or_else(|| "unknown".to_string());

    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Interface,
        &node,
        Some(sig),
        parent_index,
    ));

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// declSection: type sections containing record / class / interface / proc
// Also handles record_type → Struct
// ---------------------------------------------------------------------------

fn extract_section(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Check if this section is a record type definition by looking for kRecord keyword.
    let has_record = has_keyword_child(node, "kRecord");

    if has_record {
        let name = find_decl_type_name(node, src)
            .unwrap_or_else(|| "unknown".to_string());
        let sig = first_line_of(node, src);
        let idx = symbols.len();
        symbols.push(make_symbol(
            name.clone(),
            name,
            SymbolKind::Struct,
            &node,
            Some(sig),
            parent_index,
        ));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dispatch(child, src, symbols, refs, Some(idx));
        }
    } else {
        // General section (type, var, const, implementation, etc.) — just recurse.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dispatch(child, src, symbols, refs, parent_index);
        }
    }
}

// ---------------------------------------------------------------------------
// uses <unit1>, <unit2>;  →  Imports
// ---------------------------------------------------------------------------

fn extract_uses(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "moduleName" {
            let name = node_text(child, src);
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name.clone(),
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(name),
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// exprCall  →  Calls
// ---------------------------------------------------------------------------

fn extract_call(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    // First child of exprCall is the callee expression.
    if let Some(callee) = node.child(0) {
        let name = match callee.kind() {
            "identifier" => node_text(callee, src),
            "exprDot" | "genericDot" => {
                // Qualified call: take the last segment.
                node_text(callee, src)
            }
            _ => return,
        };
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_identifier_child(node: Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "moduleName") {
            return Some(node_text(child, src));
        }
    }
    None
}

/// For type declarations (class, interface, record): the name is typically
/// the identifier child of the containing `type` block. Walk up one level
/// or look for a varDef / declType wrapping node.
/// Simplified: look for first identifier child of the node itself.
fn find_decl_type_name(node: Node, src: &str) -> Option<String> {
    // Try named child "name" field first.
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, src));
    }
    find_identifier_child(node, src)
}

fn has_keyword_child(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

fn first_line_of(node: Node, src: &str) -> String {
    let text = node_text(node, src);
    text.lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
