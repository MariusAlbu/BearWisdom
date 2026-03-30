// =============================================================================
// parser/extractors/c_lang/mod.rs  —  C and C++ symbol and reference extractor
// =============================================================================

mod calls;
mod helpers;
mod symbols;

use calls::extract_calls_from_body;
use symbols::{
    extract_bases, extract_enum_body, push_declaration, push_function_def, push_include,
    push_namespace, push_specifier, push_typedef,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static C_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "struct_specifier", name_field: "name" },
    ScopeKind { node_kind: "enum_specifier",   name_field: "name" },
    ScopeKind { node_kind: "union_specifier",  name_field: "name" },
];

static CPP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_specifier",      name_field: "name" },
    ScopeKind { node_kind: "struct_specifier",     name_field: "name" },
    ScopeKind { node_kind: "enum_specifier",       name_field: "name" },
    ScopeKind { node_kind: "union_specifier",      name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str, language: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = if language == "c" {
        tree_sitter_c::LANGUAGE.into()
    } else {
        tree_sitter_cpp::LANGUAGE.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load C/C++ grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_config = if language == "c" { C_SCOPE_KINDS } else { CPP_SCOPE_KINDS };
    let scope_tree = scope_tree::build(root, src, scope_config);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, language, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "preproc_include" => {
                push_include(&child, src, symbols.len(), refs);
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, language, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "type_definition" => {
                push_typedef(&child, src, scope_tree, symbols, parent_index);
            }

            "struct_specifier" | "union_specifier" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Struct, symbols, parent_index);
                if language != "c" {
                    if let Some(sym_idx) = idx {
                        extract_bases(&child, src, sym_idx, refs);
                    }
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "enum_specifier" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_enum_body(&body, src, scope_tree, symbols, idx);
                }
            }

            "class_specifier" if language != "c" => {
                let idx = push_specifier(&child, src, scope_tree, SymbolKind::Class, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_bases(&child, src, sym_idx, refs);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "namespace_definition" if language != "c" => {
                let idx = push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, language, symbols, refs, idx);
                }
            }

            "declaration" | "field_declaration" => {
                push_declaration(&child, src, scope_tree, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, language, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "../c_lang_tests.rs"]
mod tests;
