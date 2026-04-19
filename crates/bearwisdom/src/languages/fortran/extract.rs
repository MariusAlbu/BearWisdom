// =============================================================================
// languages/fortran/extract.rs — Fortran extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `subroutine` (name from `subroutine_statement.name` field)
//   Function  — `function`  (name from `function_statement.name` field)
//   Function  — `program`   (name from `program_statement.name` child)
//   Namespace — `module`    (name from `module_statement.name` child)
//   Namespace — `submodule` (name from `submodule_statement.name` child)
//   Struct    — `derived_type_definition` (name from `derived_type_statement`)
//   Variable  — `variable_declaration` at module/program/submodule scope
//
// REFERENCES:
//   Imports     — `use_statement` → `module_name` child
//   Calls       — `subroutine_call` → `subroutine` field
//   Calls       — `call_expression` → `function` field
//   Inherits    — `derived_type_statement` `base` field (EXTENDS clause)
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
        "program" => {
            // PROGRAM name ... END PROGRAM name — main entry point → Function
            let name = find_program_name(node, src);
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
        "submodule" => {
            // SUBMODULE (ancestor[:parent]) name — scoped namespace
            let name = find_submodule_name(node, src);
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
                // Emit Inherits edge for EXTENDS(base_type) if present
                extract_extends(node, src, idx, refs);
                walk_children(node, src, symbols, refs, Some(idx));
            } else {
                walk_children(node, src, symbols, refs, parent_idx);
            }
        }
        "variable_declaration" => {
            // Emit Variable symbols only at module/program/submodule scope
            // (parent_idx points to a Namespace/Function entry point).
            // Skip inside subroutines/functions to avoid local variable noise.
            if let Some(sym_idx) = parent_idx {
                let sym_kind = symbols.get(sym_idx).map(|s| s.kind);
                if matches!(sym_kind, Some(SymbolKind::Namespace)) {
                    extract_variable_declaration(node, src, sym_idx, symbols, parent_idx);
                }
            }
            // No walk_children — variable_declaration has no nested scopes.
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
                            byte_offset: 0,
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
                        byte_offset: 0,
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
                        // Skip string literals misidentified as callees (e.g. "$fpm").
                        if !name.is_empty() && !name.starts_with('"') && !name.starts_with('\'') {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
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
                                    byte_offset: 0,
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
                                    byte_offset: 0,
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

fn find_program_name(node: Node, src: &[u8]) -> Option<String> {
    // program_statement has a single `name` child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "program_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = text(gc, src);
                    if !n.is_empty() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

fn find_submodule_name(node: Node, src: &[u8]) -> Option<String> {
    // submodule_statement: `name` child is the submodule identifier;
    // `ancestor` field is the parent module name (not our symbol name).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "submodule_statement" {
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "name" {
                    let n = text(gc, src);
                    if !n.is_empty() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Emit Inherits edge(s) from `derived_type_statement.base` field (EXTENDS clause).
/// base_type_specifier has a single `identifier` child that is the base type name.
fn extract_extends(
    node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "derived_type_statement" {
            // Iterate `base` field children (base_type_specifier nodes)
            let mut c2 = child.walk();
            for gc in child.children(&mut c2) {
                if gc.kind() == "base_type_specifier" {
                    // base_type_specifier → single identifier child
                    let mut c3 = gc.walk();
                    for ggc in gc.children(&mut c3) {
                        if ggc.kind() == "identifier" {
                            let base_name = text(ggc, src);
                            if !base_name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: base_name,
                                    kind: EdgeKind::Inherits,
                                    line: gc.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Extract Variable symbols from a variable_declaration node.
/// Iterates `declarator` field entries; handles `identifier` and `init_declarator`.
fn extract_variable_declaration(
    node: Node,
    src: &[u8],
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // declarator field values: identifier | init_declarator | sized_declarator | ...
        let var_name = match child.kind() {
            "identifier" => text(child, src),
            "init_declarator" => {
                // left field = identifier | sized_declarator | coarray_declarator
                child.child_by_field_name("left")
                    .map(|n| text(n, src))
                    .unwrap_or_default()
            }
            "sized_declarator" => {
                // first named child is the identifier
                child.named_child(0).map(|n| text(n, src)).unwrap_or_default()
            }
            _ => continue,
        };
        if var_name.is_empty() {
            continue;
        }
        symbols.push(ExtractedSymbol {
            qualified_name: var_name.clone(),
            name: var_name,
            kind: SymbolKind::Variable,
            visibility: Some(Visibility::Public),
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: parent_idx,
        });
        let _ = source_symbol_index; // used for scope association via parent_idx
    }
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
