// =============================================================================
// parser/extractors/python/mod.rs  —  Python symbol and reference extractor
// =============================================================================


use super::{calls, symbols};
use crate::types::{EdgeKind, ExtractionResult};
use crate::types::{ExtractedRef, ExtractedSymbol};
use super::helpers::node_text;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------



/// Extract all symbols and references from Python source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_python::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Python grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
                has_errors: true,
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(tree.root_node(), source, &mut syms, &mut refs, None, "", false);

    let has_errors = tree.root_node().has_error();
    ExtractionResult::new(syms, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                symbols::extract_function_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    &[],
                );
            }

            "class_definition" => {
                symbols::extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "decorated_definition" => {
                symbols::extract_decorated_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }

            "import_statement" => {
                calls::extract_import_statement(&child, source, refs, symbols.len());
            }

            "import_from_statement" => {
                calls::extract_import_from_statement(&child, source, refs, symbols.len());
            }

            // `type Point = tuple[int, int]` (Python 3.12+)
            "type_alias_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                symbols::extract_type_alias_top_level(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                );
            }

            "expression_statement" => {
                symbols::extract_assignment_if_any(
                    &child,
                    source,
                    symbols,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
                // Also extract any call expressions inside the statement (e.g.
                // `foo()` or `bar.baz()` at module/class body level).
                calls::extract_calls_from_body(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                );
                // Extract TypeRef from variable type annotations:
                // `items: List[str] = []` — the `assignment.type` field.
                extract_annotation_type_refs(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            // `foo()` / `bar.baz()` at module or class body level.
            // `call` can also appear as a direct child when not wrapped in
            // `expression_statement` (rare but possible in some parse trees).
            "call" => {
                calls::extract_calls_from_body(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            // `with open('f') as fh:` — context manager
            "with_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                symbols::extract_with_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                );
            }

            // `match command: case ...:` — structural pattern matching (3.10+)
            "match_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                symbols::extract_match_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Annotation TypeRef helper
// ---------------------------------------------------------------------------

/// Walk an `expression_statement` for annotated assignments and emit a
/// `TypeRef` edge for the type annotation.
///
/// ```python
/// items: List[str] = []      # assignment with `type` field
/// count: int                  # bare annotation (no value)
/// ```
fn extract_annotation_type_refs(
    expr_stmt: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = expr_stmt.walk();
    for child in expr_stmt.children(&mut cursor) {
        if child.kind() == "assignment" {
            if let Some(type_node) = child.child_by_field_name("type") {
                emit_type_ref_from_annotation(&type_node, source, source_symbol_index, refs);
            }
        }
    }
}

fn emit_type_ref_from_annotation(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty()
                && !matches!(name.as_str(), "None" | "int" | "str" | "float" | "bool" | "bytes")
            {
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
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    emit_type_ref_from_annotation(&child, source, source_symbol_index, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

