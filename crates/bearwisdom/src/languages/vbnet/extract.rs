// =============================================================================
// languages/vbnet/extract.rs — VB.NET extractor (tree-sitter-based)
//
// SYMBOLS:
//   Class     — `class_block` (name field)
//   Class     — `module_block` (VB Module = static class)
//   Struct    — `structure_block`
//   Interface — `interface_block`
//   Enum      — `enum_block`
//   Method    — `method_declaration` (Sub or Function)
//   Property  — `property_declaration`
//   Namespace — `namespace_block`
//
// REFERENCES:
//   Imports   — `imports_statement` → namespace field
//   Inherits  — `inherits_clause` → type children
//   Calls     — `invocation` / `new_expression`
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_vb_dotnet::LANGUAGE.into())
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
        "class_block" => {
            let idx = push_named(node, src, SymbolKind::Class, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "module_block" => {
            let idx = push_named(node, src, SymbolKind::Class, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "structure_block" => {
            let idx = push_named(node, src, SymbolKind::Struct, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "interface_block" => {
            let idx = push_named(node, src, SymbolKind::Interface, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "enum_block" => {
            let idx = push_named(node, src, SymbolKind::Enum, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "enum_member" => {
            // Direct child of enum_block; has a `name` field.
            push_named(node, src, SymbolKind::EnumMember, symbols, parent_idx);
            // No children of interest inside an enum_member.
        }
        "constructor_declaration" => {
            // `Sub New` — no `name` field in the grammar; name is always "New".
            let vis = visibility_from_modifiers(node, src);
            let idx = symbols.len();
            symbols.push(ExtractedSymbol {
                qualified_name: "New".to_string(),
                name: "New".to_string(),
                kind: SymbolKind::Constructor,
                visibility: Some(vis),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index: parent_idx,
            });
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "const_declaration" => {
            // `Const MAX_RETRY As Integer = 3` — has a `name` field.
            let idx = push_named(node, src, SymbolKind::Variable, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "delegate_declaration" => {
            // `Delegate Function Transformer(x As Integer) As Integer` — has `name` field.
            let idx = push_named(node, src, SymbolKind::Delegate, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "event_declaration" => {
            // `Event Clicked As EventHandler` — has `name` field.
            let idx = push_named(node, src, SymbolKind::Event, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "method_declaration" => {
            let idx = push_named(node, src, SymbolKind::Method, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "property_declaration" => {
            let idx = push_named(node, src, SymbolKind::Property, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "namespace_block" => {
            let idx = push_named(node, src, SymbolKind::Namespace, symbols, parent_idx);
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "imports_statement" => {
            // `Imports System.Collections.Generic`
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(ns_node) = node.child_by_field_name("namespace") {
                let name = text(ns_node, src);
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
            } else {
                // Fallback: take full text of first child that looks like a namespace
                let raw = text(node, src);
                let name: String = raw
                    .strip_prefix("Imports ")
                    .or_else(|| raw.strip_prefix("imports "))
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
                    .collect();
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
            }
        }
        "inherits_clause" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // Children with type identifier = the inherited type
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let name = text(child, src);
                    if !name.is_empty() && name != "Inherits" {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::Inherits,
                            line: node.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                        });
                    }
                }
            }
        }
        "invocation" => {
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(target) = node.child_by_field_name("target") {
                let name = leaf_name(target, src);
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
        "new_expression" => {
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(ty) = node.child_by_field_name("type") {
                let name = text(ty, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::Instantiates,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        // The grammar parses `Inherits BaseClass` inside a class body as a
        // field_declaration with declarator name "Inherits" and an ERROR node
        // holding the actual base type. Detect this pattern here; otherwise
        // extract as a Field symbol.
        "field_declaration" => {
            let sym_idx = parent_idx.unwrap_or(0);
            if let Some(base) = inherits_base_from_field_decl(node, src) {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: base,
                    kind: EdgeKind::Inherits,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            } else {
                // Ordinary field: `Private _timeout As Integer`
                // Name lives in the child variable_declarator's `name` field.
                let name = field_name_from_declarator(node, src);
                if !name.is_empty() {
                    let vis = visibility_from_modifiers(node, src);
                    symbols.push(ExtractedSymbol {
                        qualified_name: name.clone(),
                        name,
                        kind: SymbolKind::Field,
                        visibility: Some(vis),
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: None,
                        doc_comment: None,
                        scope_path: None,
                        parent_index: parent_idx,
                    });
                }
                walk_children(node, src, symbols, refs, parent_idx);
            }
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

/// Push a named symbol from a node with a `name` field. Returns the symbol index.
fn push_named(
    node: Node,
    src: &[u8],
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> usize {
    let name = node
        .child_by_field_name("name")
        .map(|n| text(n, src))
        .unwrap_or_default();
    let name = if name.is_empty() {
        format!("<anon_{}>", node.start_position().row + 1)
    } else {
        name
    };

    let vis = visibility_from_modifiers(node, src);
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
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
    });
    idx
}

fn visibility_from_modifiers(node: Node, src: &[u8]) -> Visibility {
    if let Some(mods) = node.child_by_field_name("modifiers") {
        let m = text(mods, src).to_ascii_lowercase();
        if m.contains("private") || m.contains("friend") {
            return Visibility::Private;
        }
    }
    Visibility::Public
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

fn leaf_name(node: Node, src: &[u8]) -> String {
    // For member_access: take the rightmost identifier
    if node.child_count() == 0 {
        return text(node, src);
    }
    // Walk to find last named leaf
    let mut cursor = node.walk();
    let mut last = String::new();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            last = text(child, src);
        }
    }
    last
}

/// Extract the field name from a `field_declaration`'s `variable_declarator` child.
///
/// Grammar: `field_declaration > variable_declarator [name = identifier]`
fn field_name_from_declarator(node: Node, src: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = text(name_node, src);
                // Exclude the Inherits/Implements pseudo-fields
                if name != "Inherits" && name != "Implements" {
                    return name;
                }
            }
        }
    }
    String::new()
}

/// Detect the grammar's encoding of `Inherits BaseClass` / `Implements IFoo`.
///
/// The tree-sitter-vb-dotnet grammar parses `Inherits X` inside a class body as:
///   field_declaration
///     variable_declarator[name = "Inherits" | "Implements"]
///     ERROR
///       identifier = <base type name>
///
/// Returns the base type name when the pattern matches.
fn inherits_base_from_field_decl(node: Node, src: &[u8]) -> Option<String> {
    // First child should be a variable_declarator whose name field == "Inherits"/"Implements"
    let mut cursor = node.walk();
    let mut is_inherits_decl = false;
    let mut error_ident: Option<String> = None;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_declarator" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = text(name_node, src);
                    if name == "Inherits" || name == "Implements" {
                        is_inherits_decl = true;
                    }
                }
            }
            "ERROR" => {
                // The base type identifier lives inside the ERROR node
                let mut ec = child.walk();
                for err_child in child.children(&mut ec) {
                    if err_child.kind() == "identifier" {
                        error_ident = Some(text(err_child, src));
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    if is_inherits_decl {
        error_ident
    } else {
        None
    }
}
