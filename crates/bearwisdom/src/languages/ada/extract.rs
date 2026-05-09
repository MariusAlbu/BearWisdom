// =============================================================================
// languages/ada/extract.rs — Ada extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `subprogram_declaration` and `subprogram_body`
//               (inner `function_specification` or `procedure_specification`)
//   Namespace — `package_declaration` and `package_body` (name field)
//   Struct    — `full_type_declaration` with record body
//   Enum      — `full_type_declaration` with enumeration body
//
// REFERENCES:
//   Imports   — `with_clause` → identifier children
//   Calls     — `procedure_call_statement` and `function_call`
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_ada::LANGUAGE.into())
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
        "subprogram_declaration" | "subprogram_body" => {
            let idx = extract_subprogram(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "package_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "package_body" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Namespace, symbols, parent_idx);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "full_type_declaration" => {
            let idx = extract_type_decl(node, src, symbols, parent_idx);
            walk_children(node, src, symbols, refs, idx.or(parent_idx));
        }
        "package_renaming_declaration" => {
            // `package Trace renames Simple_Logging;` brings Simple_Logging
            // into scope as `Trace`. Without this, every `Trace.<x>` call
            // references an undefined symbol (Simple_Logging is typically
            // an external Ada library — Alire's lib uses `Trace renames
            // Simple_Logging` for ~600 unresolveds in alire).
            //
            // tree-sitter-ada emits the rename as:
            //   package identifier "<alias>" renames <identifier|selected_component> ;
            let sym_idx = parent_idx.unwrap_or(0);
            let mut cursor = node.walk();
            let mut alias_name: Option<String> = None;
            let mut target_module: Option<String> = None;
            let mut seen_renames = false;
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "renames" => seen_renames = true,
                    "identifier" => {
                        if !seen_renames {
                            alias_name = Some(text(child, src));
                        } else {
                            target_module = Some(text(child, src));
                        }
                    }
                    "selected_component" => {
                        if seen_renames {
                            target_module = Some(text(child, src));
                        }
                    }
                    _ => {}
                }
            }
            if let (Some(alias), Some(target)) = (alias_name, target_module) {
                if !alias.is_empty() && !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: alias,
                        kind: EdgeKind::Imports,
                        line: node.start_position().row as u32,
                        module: Some(target),
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }
        "with_clause" | "use_clause" | "use_type_clause" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // `with X;` makes X visible dot-qualified. `use X;` brings X's
            // exports into bare scope; `use type X;` only brings primitive
            // operators of type X into scope. All three produce Imports edges
            // so the resolver's FileContext sees them as wildcard candidates.
            // Children include `identifier` (simple) and `selected_component`
            // (dotted: Ada.Text_IO) nodes for each package name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => {
                        let name = text(child, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Imports,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                        }
                    }
                    "selected_component" => {
                        // Use the full text (e.g. "Ada.Text_IO") as module name
                        let name = text(child, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Imports,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                        }
                    }
                    _ => {}
                }
            }
        }
        "procedure_call_statement" => {
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
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
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
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx);
        }
    }
}

fn extract_subprogram(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let name = match child.kind() {
            "function_specification" | "procedure_specification" => {
                child
                    .child_by_field_name("name")
                    .map(|n| text(n, src))
            }
            _ => None,
        };
        if let Some(name) = name {
            if !name.is_empty() {
                let idx = push_sym(node, name, SymbolKind::Function, symbols, parent_idx);
                return Some(idx);
            }
        }
    }
    None
}

fn extract_type_decl(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    // Gather identifiers (name) and determine kind from body
    let mut name = String::new();
    let mut kind = SymbolKind::Struct;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" if name.is_empty() => {
                name = text(child, src);
            }
            "enumeration_type_definition" => {
                kind = SymbolKind::Enum;
            }
            "record_type_definition" => {
                kind = SymbolKind::Struct;
            }
            _ => {}
        }
    }

    if name.is_empty() { return None; }
    let idx = push_sym(node, name, kind, symbols, parent_idx);
    Some(idx)
}

fn push_sym(
    node: Node,
    name: String,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> usize {
    let idx = symbols.len();
    // Qualify by the parent symbol's qualified_name when one exists. Ada is
    // package-scoped: `package Trace is procedure Debug ...` ⇒ Debug must
    // be reachable via the qname `Trace.Debug` so cross-file callers like
    // `Trace.Debug(msg)` resolve via the engine's qualified_name lookup
    // (Step 5 in resolve_common).
    let qualified_name = match parent_idx.and_then(|i| symbols.get(i)) {
        Some(parent) if !parent.qualified_name.is_empty() => {
            format!("{}.{}", parent.qualified_name, name)
        }
        _ => name.clone(),
    };
    symbols.push(ExtractedSymbol {
        qualified_name,
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
