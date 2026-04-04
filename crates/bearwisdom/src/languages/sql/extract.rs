// =============================================================================
// languages/sql/extract.rs  —  SQL schema extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct    — CREATE TABLE (tables are structs; columns are fields)
//   Class     — CREATE VIEW (views are class-like)
//   Function  — CREATE FUNCTION / CREATE TRIGGER
//   Variable  — CREATE INDEX
//   Field     — column_definition (under parent table/view scope)
//
// REFERENCES:
//   TypeRef   — ALTER TABLE → referenced table name
//   TypeRef   — foreign key REFERENCES clause → referenced table
//   TypeRef   — column type names (custom types)
//
// Grammar: tree-sitter-sequel 0.3.x
//   Key node types:
//     create_table, create_view, create_function, create_trigger, create_index
//     column_definitions → column_definition{name, type fields}
//     object_reference{name field}
//     alter_table
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> crate::types::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_sequel::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load SQL grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_root(tree.root_node(), source, &mut symbols, &mut refs);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Root-level traversal
// ---------------------------------------------------------------------------

fn visit_root(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "create_table" => extract_create_table(&child, src, symbols, refs),
            "create_view" => extract_create_view(&child, src, symbols, refs),
            "create_function" => extract_create_function(&child, src, symbols, refs),
            "create_trigger" => extract_create_trigger(&child, src, symbols, refs),
            "create_index" => extract_create_index(&child, src, symbols, refs),
            "alter_table" => extract_alter_table(&child, src, symbols.len(), refs),
            "statement" => {
                // sequel wraps statements in a `statement` node
                visit_root(child, src, symbols, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// CREATE TABLE
// ---------------------------------------------------------------------------

fn extract_create_table(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match first_object_reference_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Struct,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("CREATE TABLE {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // Extract column definitions as Field children
    extract_column_definitions(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// CREATE VIEW
// ---------------------------------------------------------------------------

fn extract_create_view(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // create_view uses object_reference for the view name
    let name = match first_object_reference_name(node, src) {
        Some(n) => n,
        None => {
            // Fallback: first identifier child
            match first_child_of_kind(node, "identifier")
                .map(|n| node_text(n, src))
            {
                Some(n) if !n.is_empty() => n,
                _ => return,
            }
        }
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("CREATE VIEW {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// CREATE FUNCTION / PROCEDURE
// ---------------------------------------------------------------------------

fn extract_create_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
) {
    let name = match first_object_reference_name(node, src) {
        Some(n) => n,
        None => return,
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("CREATE FUNCTION {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// CREATE TRIGGER
// ---------------------------------------------------------------------------

fn extract_create_trigger(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
) {
    // create_trigger: first identifier child after the TRIGGER keyword is the name
    let name = match first_child_of_kind(node, "identifier").map(|n| node_text(n, src)) {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("CREATE TRIGGER {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// CREATE INDEX
// ---------------------------------------------------------------------------

fn extract_create_index(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // tree-sitter-sequel grammar for CREATE INDEX:
    //   create_index → keyword_create keyword_index identifier ON object_reference index_fields
    // The index name is a bare `identifier`; the table name is the `object_reference`.
    let name = match first_child_of_kind(node, "identifier").map(|n| node_text(n, src)) {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("CREATE INDEX {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // TypeRef to the table the index is on (object_reference child)
    if let Some(table_name) = first_object_reference_name(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: table_name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// ALTER TABLE
// ---------------------------------------------------------------------------

fn extract_alter_table(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // alter_table: first object_reference is the table being altered
    if let Some(name) = first_object_reference_name(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Column definitions
// ---------------------------------------------------------------------------

fn extract_column_definitions(
    parent_node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Find column_definitions child, then iterate column_definition nodes
    let mut cursor = parent_node.walk();
    for child in parent_node.children(&mut cursor) {
        if child.kind() == "column_definitions" {
            extract_columns_from_list(&child, src, parent_index, symbols, refs);
        }
    }
}

fn extract_columns_from_list(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "column_definition" {
            extract_column(&child, src, parent_index, symbols, refs);
        }
    }
}

fn extract_column(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match node.child_by_field_name("name").map(|n| node_text(n, src)) {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    // Gather type text from the `type` field (may be a keyword_* or identifier)
    let type_text = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src).to_uppercase())
        .unwrap_or_default();

    // custom_type field holds user-defined type references
    let custom_type = node
        .child_by_field_name("custom_type")
        .and_then(|n| object_reference_name(&n, src));

    let col_idx = symbols.len();
    let sig = if type_text.is_empty() {
        name.clone()
    } else {
        format!("{name} {type_text}")
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Field,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });

    // TypeRef for custom type
    if let Some(ct) = custom_type {
        refs.push(ExtractedRef {
            source_symbol_index: col_idx,
            target_name: ct,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }

    // Check for REFERENCES clause (foreign key inline constraint)
    extract_fk_refs(node, src, col_idx, refs);
}

/// Scan a column_definition for an inline REFERENCES clause.
///
/// tree-sitter-sequel emits FK references as:
///   column_definition → … keyword_references object_reference …
/// There is no intermediate `constraint` or `foreign_key_reference` wrapper.
fn extract_fk_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk children looking for `keyword_references`; the immediately
    // following `object_reference` sibling is the referenced table.
    let mut saw_references = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "keyword_references" {
            saw_references = true;
        } else if saw_references && child.kind() == "object_reference" {
            if let Some(name) = object_reference_name(&child, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
            saw_references = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the `name` field from the first `object_reference` child.
fn first_object_reference_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "object_reference" {
            return object_reference_name(&child, src);
        }
    }
    None
}

/// Extract the `name` field from an `object_reference` node.
fn object_reference_name(node: &Node, src: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| node_text(n, src))
        .filter(|s| !s.is_empty())
}

fn first_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
