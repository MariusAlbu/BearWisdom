// =============================================================================
// parser/extractors/scala/mod.rs  —  Scala symbol and reference extractor
// =============================================================================

mod calls;
mod helpers;
mod symbols;

use calls::extract_calls_from_body;
use helpers::classify_class;
use symbols::{
    extract_enum_body, extract_extends_with, push_function_def, push_import, push_type_def,
    push_val_var, recurse_body,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static SCALA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_definition",    name_field: "name" },
    ScopeKind { node_kind: "object_definition",   name_field: "name" },
    ScopeKind { node_kind: "trait_definition",    name_field: "name" },
    ScopeKind { node_kind: "enum_definition",     name_field: "name" },
    ScopeKind { node_kind: "function_definition", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Scala grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, SCALA_SCOPE_KINDS);

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
            "import_declaration" => {
                push_import(&child, src, symbols.len(), refs);
            }

            "class_definition" => {
                let kind = classify_class(&child, src);
                let idx = push_type_def(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "object_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Namespace, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "trait_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Scala 3 enum
            "enum_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                extract_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "val_definition" | "var_definition" => {
                push_val_var(&child, src, scope_tree, symbols, parent_index);
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
