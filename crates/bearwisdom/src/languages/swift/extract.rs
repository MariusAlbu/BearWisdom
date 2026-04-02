// =============================================================================
// parser/extractors/swift/mod.rs  —  Swift symbol and reference extractor
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use super::calls::extract_calls_from_body;
use super::decorators::{
    extract_decorators, extract_extension_conformances, extract_guard_bindings,
    extract_switch_patterns,
};
use super::helpers::find_child_by_kind;
use super::symbols::{
    extract_type_inheritance, handle_class_declaration, push_deinit, push_extension,
    push_function_decl, push_import, push_init, push_property, push_subscript,
    push_type_decl, push_typealias, recurse_into_body,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static SWIFT_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "struct_declaration",   name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",     name_field: "name" },
    ScopeKind { node_kind: "protocol_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_swift::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Swift grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, SWIFT_SCOPE_KINDS);

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

            "class_declaration" => {
                // Capture the index before handle_class_declaration pushes anything.
                // push_type_decl inside it pushes the class as the first symbol, so
                // pre_len is the correct index for the class regardless of how many
                // nested symbols get pushed during recursion.
                let pre_len = symbols.len();
                handle_class_declaration(&child, src, scope_tree, symbols, refs, parent_index);
                // If a symbol was pushed it's at pre_len.
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }

            "struct_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Struct, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "enum_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                symbols::recurse_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "protocol_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_type_inheritance(&child, src, sym_idx, refs, true);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "extension_declaration" => {
                let idx = push_extension(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extension_conformances(&child, src, sym_idx, refs);
                }
                recurse_into_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "function_declaration" => {
                let idx = push_function_decl(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    } else if let Some(body) = find_child_by_kind(&child, "code_block") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "initializer_declaration" => {
                let idx = push_init(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    if let Some(body) = find_child_by_kind(&child, "code_block") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "deinit_declaration" => {
                push_deinit(&child, src, scope_tree, symbols, parent_index);
            }

            "guard_statement" => {
                if let Some(sym_idx) = parent_index {
                    extract_guard_bindings(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "switch_statement" => {
                if let Some(sym_idx) = parent_index {
                    extract_switch_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "property_declaration" | "stored_property" | "variable_declaration" => {
                let pre_len = symbols.len();
                push_property(&child, src, scope_tree, symbols, parent_index);
                if symbols.len() > pre_len {
                    extract_decorators(&child, src, pre_len, refs);
                }
            }

            "typealias_declaration" => {
                push_typealias(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "subscript_declaration" => {
                let idx = push_subscript(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Recurse into the computed_property body for calls.
                    let body = find_child_by_kind(&child, "computed_property")
                        .or_else(|| find_child_by_kind(&child, "code_block"));
                    if let Some(b) = body {
                        extract_calls_from_body(&b, src, sym_idx, refs);
                    }
                }
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

