// =============================================================================
// parser/extractors/python/mod.rs  —  Python symbol and reference extractor
// =============================================================================


use super::{calls, symbols};
use crate::parser::extractors::ExtractionResult;
use crate::types::{ExtractedRef, ExtractedSymbol};
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
                has_errors: true,
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(tree.root_node(), source, &mut syms, &mut refs, None, "", false);

    let has_errors = tree.root_node().has_error();
    ExtractionResult { symbols: syms, refs, has_errors }
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
// Tests
// ---------------------------------------------------------------------------

