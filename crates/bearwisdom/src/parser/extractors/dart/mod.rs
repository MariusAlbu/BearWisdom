// =============================================================================
// parser/extractors/dart/mod.rs  —  Dart symbol and reference extractor
// =============================================================================

mod calls;
mod helpers;
mod symbols;

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
                extract_class(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "mixin_declaration" => {
                extract_mixin(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "extension_declaration" => {
                extract_extension(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "enum_declaration" => {
                extract_enum(&child, src, symbols, parent_index, qualified_prefix);
            }
            "function_signature" | "function_declaration" => {
                if parent_index.is_none() {
                    extract_top_level_function(&child, src, symbols, parent_index, qualified_prefix);
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
#[path = "../dart_tests.rs"]
mod tests;
