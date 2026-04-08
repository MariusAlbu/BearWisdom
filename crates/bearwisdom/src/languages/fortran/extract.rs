// =============================================================================
// languages/fortran/extract.rs — Fortran extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `subroutine` (name from `subroutine_statement.name` field)
//   Function  — `function`  (name from `function_statement.name` field)
//   Namespace — `module`    (name from `module_statement.name` child)
//   Struct    — `derived_type_definition` (name from `derived_type_statement`)
//
// REFERENCES:
//   Imports   — `use_statement` → `module_name` child
//   Calls     — `subroutine_call` → `subroutine` field
//   Calls     — `call_expression` → `function` field
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_fortran::LANGUAGE.into())
        .is_err()
    {
        return ExtractionResult::empty();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = source.as_bytes();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    match node.kind() {
        "subroutine" => {
            let name = find_child_name(node, src, "subroutine_statement");
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "function" => {
            let name = find_child_name(node, src, "function_statement");
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "module" => {
            let name = find_module_name(node, src);
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "derived_type_definition" => {
            let name = find_derived_type_name(node, src);
            let name = name.unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Struct, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "use_statement" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // `module_name` child holds the module name
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "module_name" || child.kind() == "name" {
                    let name = text(child, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line: node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                    break;
                }
            }
        }
        "subroutine_call" => {
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(sub_node) = node.child_by_field_name("subroutine") {
                let name = text(sub_node, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        "call_expression" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // call_expression = _expression REPEAT1(argument_list)
            // The grammar has no named field; the callee is the first child.
            if let Some(callee) = node.child(0) {
                match callee.kind() {
                    "identifier" => {
                        let name = text(callee, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    // derived_type_member_expression: obj%method
                    // named children: [0] = object, [last] = method name
                    "derived_type_member_expression" => {
                        let count = callee.named_child_count();
                        if count >= 2 {
                            let obj_text = callee.named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            let method_text = callee.named_child(count - 1)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if !method_text.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: method_text,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    module: if obj_text.is_empty() { None } else { Some(obj_text) },
                                    chain: None,
                                });
                            }
                        } else if count == 1 {
                            // Single named child — use as target_name, no module
                            let name = callee.named_child(0)
                                .map(|n| text(n, src))
                                .unwrap_or_default();
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: name,
                                    kind: EdgeKind::Calls,
                                    line: node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx);
        }
    }
}

/// Find the `name` field within a named child of the given kind.
/// E.g., `find_child_name(subroutine_node, "subroutine_statement")` returns
/// the name of the subroutine.
fn find_child_name(node: Node, src: &[u8], child_kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == child_kind {
            if let Some(name_node) = child.child_by_field_name("name") {
                let n = text(name_node, src);
                if !n.is_empty() { return Some(n); }
            }
            // Fallback: first `name` child
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = text(gc, src);
                    if !n.is_empty() { return Some(n); }
                }
            }
        }
    }
    None
}

fn find_module_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = text(gc, src);
                    if !n.is_empty() { return Some(n); }
                }
            }
        }
    }
    None
}

fn find_derived_type_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "derived_type_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "type_name" {
                    let n = text(gc, src);
                    if !n.is_empty() { return Some(n); }
                }
            }
        }
    }
    None
}

fn push_sym(
    node: Node,
    name: String,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> usize {
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
    });
    idx
}

fn walk_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx);
    }
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").trim().to_string()
}
