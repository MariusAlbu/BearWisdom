// =============================================================================
// languages/matlab/extract.rs — MATLAB extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `function_definition` (name field)
//   Class     — `class_definition` (name field)
//   Variable  — top-level `assignment` (simple identifier on left)
//
// REFERENCES:
//   Calls     — `function_call` / identifier-like calls
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

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None, true);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    top_level: bool,
) {
    match node.kind() {
        "source_file" => {
            // Propagate top_level=true to direct children of the root
            walk_children_with_level(node, src, symbols, refs, parent_idx, true);
        }
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
            symbols.push(make_sym(
                name, SymbolKind::Function, Visibility::Public,
                node, parent_idx,
            ));
            walk_children_with_level(node, src, symbols, refs, Some(idx), false);
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
            symbols.push(make_sym(
                name, SymbolKind::Class, Visibility::Public,
                node, parent_idx,
            ));
            walk_children_with_level(node, src, symbols, refs, Some(idx), false);
        }
        "assignment" if top_level => {
            // Only capture simple `var = expr` at top level as Variable.
            // The grammar uses field name "left" for the LHS, not "variable".
            if let Some(lhs) = node.child_by_field_name("left") {
                let name = text(lhs, src);
                if !name.is_empty() && is_simple_ident(&name) {
                    symbols.push(make_sym(
                        name, SymbolKind::Variable, Visibility::Public,
                        node, parent_idx,
                    ));
                }
            }
        }
        "function_call" => {
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, src);
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
            walk_children_with_level(node, src, symbols, refs, parent_idx, false);
        }
        _ => {
            walk_children_with_level(node, src, symbols, refs, parent_idx, false);
        }
    }
}

fn walk_children_with_level(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    top_level: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx, top_level);
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
