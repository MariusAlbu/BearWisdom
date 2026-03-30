// =============================================================================
// parser/extractors/kotlin/mod.rs  —  Kotlin symbol and reference extractor
// =============================================================================

mod calls;
mod decorators;
mod helpers;
mod symbols;

use calls::extract_calls_from_body;
use decorators::{extract_decorators, extract_lambda_params, extract_when_patterns};
use helpers::{classify_class, find_child_by_kind};
use symbols::{
    emit_import, extract_class_body, extract_delegation_specifiers, extract_imports,
    push_function_decl, push_property_decl, push_secondary_constructor, push_type_decl,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static KOTLIN_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",     name_field: "name" },
    ScopeKind { node_kind: "object_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration",  name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Kotlin grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, KOTLIN_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

pub(super) fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_list" => {
                extract_imports(&child, src, symbols.len(), refs);
            }

            "import" => {
                emit_import(&child, src, symbols.len(), refs);
            }

            "class_declaration" => {
                let kind = classify_class(&child, src);
                let idx = push_type_decl(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    extract_class_body(&child, src, scope_tree, symbols, refs, idx);
                }
            }

            "object_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Class, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_delegation_specifiers(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_node(body, src, scope_tree, symbols, refs, idx);
                    }
                }
            }

            "function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    // function_body is a child (not a named field) in kotlin-ng 1.1.
                    let body = child.child_by_field_name("body")
                        .or_else(|| find_child_by_kind(&child, "function_body"));
                    if let Some(b) = body {
                        extract_calls_from_body(&b, src, sym_idx, refs);
                        extract_lambda_params(&b, src, sym_idx, symbols);
                    }
                }
            }

            "when_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_when_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "property_declaration" => {
                push_property_decl(&child, src, scope_tree, symbols, parent_index);
            }

            "secondary_constructor" => {
                push_secondary_constructor(&child, src, scope_tree, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
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
