// =============================================================================
// parser/extractors/dart/mod.rs  —  Dart symbol and reference extractor
// =============================================================================

mod calls;
mod decorators;
mod helpers;
mod symbols;

use decorators::{extract_cascade_calls, extract_decorators};
use symbols::{
    extract_class, extract_enum, extract_extension, extract_import_directive, extract_mixin,
    extract_part_directive, extract_top_level_function, extract_variable,
};

use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Dart grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, "");

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "class_definition" => {
                let pre_len = symbols.len();
                extract_class(&child, src, symbols, refs, parent_index, qualified_prefix);
                // Annotations appear as children of the class_declaration node.
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "mixin_declaration" => {
                let pre_len = symbols.len();
                extract_mixin(&child, src, symbols, refs, parent_index, qualified_prefix);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "extension_declaration" => {
                let pre_len = symbols.len();
                extract_extension(&child, src, symbols, refs, parent_index, qualified_prefix);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "enum_declaration" => {
                let pre_len = symbols.len();
                extract_enum(&child, src, symbols, parent_index, qualified_prefix);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }
            "function_signature" | "function_declaration" => {
                if parent_index.is_none() {
                    let pre_len = symbols.len();
                    extract_top_level_function(&child, src, symbols, parent_index, qualified_prefix);
                    if symbols.len() > pre_len {
                        extract_decorators(&child, src, pre_len, refs);
                    }
                }
            }
            "import_or_export" | "library_import" => {
                extract_import_directive(&child, src, symbols.len(), refs);
            }
            "part_directive" | "part_of_directive" => {
                extract_part_directive(&child, src, symbols.len(), refs);
            }
            "initialized_variable_definition" | "static_final_declaration" => {
                if parent_index.is_none() {
                    extract_variable(&child, src, symbols, parent_index, qualified_prefix);
                }
            }
            // Cascade expressions at statement level — extract each section's calls.
            "expression_statement" | "return_statement" => {
                if let Some(sym_idx) = parent_index {
                    extract_cascade_calls(&child, src, sym_idx, refs);
                }
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "ERROR" | "MISSING" => {}
            _ => {
                visit(child, src, symbols, refs, parent_index, qualified_prefix);
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
