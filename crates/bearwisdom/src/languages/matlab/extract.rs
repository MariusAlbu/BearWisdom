// =============================================================================
// languages/matlab/extract.rs — MATLAB extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `function_definition` (name field)
//   Class     — `class_definition` (name field)
//   Variable  — `assignment` at any scope (simple identifier on left)
//
// REFERENCES:
//   Calls     — `function_call` nodes (name or field_expression)
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
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
        "function_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            let name = if name.is_empty() {
                format!("<anon_fn_{}>", node.start_position().row + 1)
            } else {
                name
            };
            let idx = symbols.len();
            symbols.push(make_sym(name, SymbolKind::Function, Visibility::Public, node, parent_idx));
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "class_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            let name = if name.is_empty() {
                format!("<anon_class_{}>", node.start_position().row + 1)
            } else {
                name
            };
            let idx = symbols.len();
            symbols.push(make_sym(name, SymbolKind::Class, Visibility::Public, node, parent_idx));
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "methods" => {
            // `methods` block inside a `class_definition` — extract nested
            // function_definition nodes as Method symbols.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_definition" {
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| text(n, src))
                        .unwrap_or_default();
                    let name = if name.is_empty() {
                        format!("<anon_method_{}>", child.start_position().row + 1)
                    } else {
                        name
                    };
                    let idx = symbols.len();
                    symbols.push(make_sym(name, SymbolKind::Method, Visibility::Public, child, parent_idx));
                    walk_children(child, src, symbols, refs, Some(idx));
                } else {
                    walk_node(child, src, symbols, refs, parent_idx);
                }
            }
        }
        "assignment" => {
            // Capture assignments at any scope as Variable symbols.
            // field "left" holds the LHS.
            if let Some(lhs) = node.child_by_field_name("left") {
                match lhs.kind() {
                    "identifier" => {
                        let name = text(lhs, src);
                        if !name.is_empty() && is_simple_ident(&name) {
                            symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                        }
                    }
                    "function_call" => {
                        // Indexed assignment: `arr(i) = val` — use the function name as symbol
                        if let Some(name_node) = lhs.child_by_field_name("name") {
                            let name = text(name_node, src);
                            if !name.is_empty() && is_simple_ident(&name) {
                                symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                            }
                        }
                    }
                    "field_expression" => {
                        // Field assignment: `obj.field = val` — use the field name
                        if let Some(field_node) = lhs.child_by_field_name("field") {
                            let name = text(field_node, src);
                            if !name.is_empty() {
                                symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                            }
                        }
                    }
                    "multioutput_variable" => {
                        // Destructuring: `[a, b] = func()` — emit each variable
                        let mut mc = lhs.walk();
                        for child in lhs.children(&mut mc) {
                            if child.kind() == "identifier" {
                                let name = text(child, src);
                                if !name.is_empty() && is_simple_ident(&name) {
                                    symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                                }
                            }
                        }
                    }
                    _ => {
                        // Fallback: emit a symbol using the whole LHS text if it looks like an ident
                        let name = text(lhs, src);
                        if !name.is_empty() && is_simple_ident(&name) {
                            symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                        }
                    }
                }
            }
            // Always recurse into children so function_call nodes on the RHS are visited.
            walk_children(node, src, symbols, refs, parent_idx);
        }
        "field_expression" => {
            // obj.method(args) — the grammar represents this as:
            //   field_expression
            //     object: identifier("obj")
            //     field:  function_call(name: identifier("method"), ...)
            // We want: target_name = "method", module = Some("obj").
            // Handle this here so we can emit module; then skip recursing into
            // children to avoid the nested function_call arm firing again.
            let sym_idx = parent_idx.unwrap_or(0);
            let object_node = node.child_by_field_name("object");
            let field_node = node.child_by_field_name("field");

            match (object_node, field_node) {
                (Some(obj), Some(field)) if field.kind() == "function_call" => {
                    let module_text = text(obj, src);
                    let method_text = field
                        .child_by_field_name("name")
                        .map(|n| text(n, src))
                        .unwrap_or_default();
                    if !method_text.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: method_text,
                            kind: EdgeKind::Calls,
                            line: node.start_position().row as u32,
                            module: if module_text.is_empty() { None } else { Some(module_text) },
                            chain: None,
                            byte_offset: 0,
                        });
                    }
                    // Recurse into the function_call's arguments but not into the
                    // call node itself (to avoid duplicate emission).
                    walk_children(field, src, symbols, refs, parent_idx);
                }
                _ => {
                    // Plain field access (not a call) — just recurse.
                    walk_children(node, src, symbols, refs, parent_idx);
                }
            }
        }
        "function_call" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // Simple function call (not a field_expression callee — those are
            // handled by the field_expression arm above).
            let target = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();

            if !target.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: target,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx);
        }
    }
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

fn make_sym(
    name: String,
    kind: SymbolKind,
    vis: Visibility,
    node: Node,
    parent_idx: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
    }
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

fn is_simple_ident(s: &str) -> bool {
    s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && s.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
}
