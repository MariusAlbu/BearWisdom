// =============================================================================
// parser/extractors/ruby/mod.rs  —  Ruby symbol and reference extractor
// =============================================================================

mod calls;
mod helpers;
mod params;
mod symbols;

use symbols::{
    extract_call_statement, extract_class, extract_method, extract_module,
    extract_singleton_method,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static RUBY_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class",            name_field: "name" },
    ScopeKind { node_kind: "module",           name_field: "name" },
    ScopeKind { node_kind: "method",           name_field: "name" },
    ScopeKind { node_kind: "singleton_method", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_ruby::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Ruby grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let src = source.as_bytes();
    let root = tree.root_node();

    // Build scope tree for qualified-name lookups (used in scope_path).
    let _scope_tree = scope_tree::build(root, src, RUBY_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_from_node(root, src, &mut symbols, &mut refs, None, "", false);

    super::ExtractionResult::new(symbols, refs, has_errors)
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
    inside_class: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" => {
                extract_class(&child, src, symbols, refs, parent_index, qualified_prefix);
            }

            "module" => {
                extract_module(&child, src, symbols, refs, parent_index, qualified_prefix);
            }

            "method" => {
                extract_method(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }

            "singleton_method" => {
                extract_singleton_method(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "call" => {
                extract_call_statement(
                    &child,
                    src,
                    refs,
                    symbols.len(),
                    parent_index,
                    inside_class,
                    symbols,
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
