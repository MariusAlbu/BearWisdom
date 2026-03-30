// =============================================================================
// parser/extractors/python/mod.rs  —  Python symbol and reference extractor
// =============================================================================

mod calls;
mod helpers;
mod symbols;

use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct PythonExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract all symbols and references from Python source code.
pub fn extract(source: &str) -> PythonExtraction {
    let language = tree_sitter_python::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Python grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return PythonExtraction {
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
    PythonExtraction { symbols: syms, refs, has_errors }
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
