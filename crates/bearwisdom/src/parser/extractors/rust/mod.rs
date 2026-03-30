// =============================================================================
// parser/extractors/rust/mod.rs  —  Rust symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct, Enum, EnumMember, Interface (trait), Method, Function,
//   TypeAlias, Variable (static), Namespace (mod), Test
//
// REFERENCES:
//   - `use` declarations     → Import edges (recursive use-tree walking)
//   - `call_expression`      → Calls edges
//
// Approach:
//   Single-pass recursive CST walk. No scope tree — qualified names are built
//   by threading a `qualified_prefix` string through the recursion. `impl`
//   blocks are not symbols themselves; they set the prefix for their methods.
// =============================================================================

mod calls;
mod helpers;
mod symbols;

use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// Re-exports required by rust_tests.rs (`use super::*`).
#[cfg(test)]
pub(crate) use crate::types::{EdgeKind, SymbolKind, Visibility};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct RustExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract all symbols and references from Rust source code.
pub fn extract(source: &str) -> RustExtraction {
    let language = tree_sitter_rust::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Rust grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return RustExtraction {
                symbols: vec![],
                refs: vec![],
                has_errors: true,
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(
        tree.root_node(),
        source,
        &mut syms,
        &mut refs,
        None,
        "",
    );

    let has_errors = tree.root_node().has_error();
    RustExtraction { symbols: syms, refs, has_errors }
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" | "function_signature_item" => {
                if let Some(sym) =
                    symbols::extract_function(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body(&body, source, idx, refs);
                    }
                }
            }

            "struct_item" => {
                if let Some(sym) =
                    symbols::extract_struct(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "enum_item" => {
                if let Some(sym) =
                    symbols::extract_enum(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        symbols::extract_enum_variants(&body, source, Some(idx), &new_prefix, symbols);
                    }
                }
            }

            "trait_item" => {
                if let Some(sym) =
                    symbols::extract_trait(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "impl_item" => {
                calls::extract_impl(&child, source, symbols, refs, qualified_prefix);
            }

            "type_item" => {
                if let Some(sym) =
                    symbols::extract_type_alias(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "const_item" => {
                if let Some(sym) =
                    symbols::extract_const(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "static_item" => {
                if let Some(sym) =
                    symbols::extract_static(&child, source, parent_index, qualified_prefix)
                {
                    symbols.push(sym);
                }
            }

            "mod_item" => {
                if let Some(sym) =
                    symbols::extract_mod(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "use_declaration" => {
                calls::extract_use_names(&child, source, refs, symbols.len());
            }

            "macro_definition" => {}

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "../rust_tests.rs"]
mod tests;
