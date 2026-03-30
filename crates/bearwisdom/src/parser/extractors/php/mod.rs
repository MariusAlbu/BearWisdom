// =============================================================================
// parser/extractors/php/mod.rs  —  PHP symbol and reference extractor
// =============================================================================

mod calls;
mod helpers;
mod symbols;

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static PHP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "trait_declaration",    name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",     name_field: "name" },
    ScopeKind { node_kind: "method_declaration",   name_field: "name" },
    ScopeKind { node_kind: "function_definition",  name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from PHP source code.
pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load PHP grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let src = source.as_bytes();
    let root = tree.root_node();

    let _scope_tree = scope_tree::build(root, src, PHP_SCOPE_KINDS);

    let mut syms: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_from_node(root, src, &mut syms, &mut refs, None, "", "");

    super::ExtractionResult::new(syms, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn extract_from_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_definition" => {
                symbols::extract_namespace(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "namespace_use_declaration" => {
                calls::extract_use_declaration(&child, src, refs, symbols.len());
            }

            "function_definition" => {
                symbols::extract_function(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    false,
                );
            }

            "class_declaration" => {
                symbols::extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Class,
                );
            }

            "interface_declaration" => {
                symbols::extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Interface,
                );
            }

            "trait_declaration" => {
                symbols::extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Class,
                );
            }

            "enum_declaration" => {
                symbols::extract_enum(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "../php_tests.rs"]
mod tests;
