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


use super::{calls, symbols, helpers, decorators, patterns};
use crate::parser::extractors::ExtractionResult;
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// Re-exports required by rust_tests.rs (`use super::*`).
pub(crate) use crate::types::{EdgeKind, SymbolKind, Visibility};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------



/// Extract all symbols and references from Rust source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_rust::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Rust grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
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
    ExtractionResult::new(syms, refs, has_errors)
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
                    decorators::extract_decorators(&child, source, idx, refs);
                    // where-clause and type-parameter bounds → TypeRef edges.
                    // Iterate children by kind rather than field_name to avoid
                    // grammar-version sensitivity.
                    {
                        let mut wc = child.walk();
                        for gc in child.children(&mut wc) {
                            match gc.kind() {
                                "type_parameters" => {
                                    patterns::extract_type_param_bounds(&gc, source, idx, refs);
                                }
                                "where_clause" => {
                                    patterns::extract_where_clause(&gc, source, idx, refs);
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body_with_symbols(&body, source, idx, refs, Some(symbols));
                    }
                }
            }

            "struct_item" | "union_item" => {
                // `union_item` has the same field layout as `struct_item` in
                // tree-sitter-rust; reuse the struct extractor and emit Struct kind.
                if let Some(sym) =
                    symbols::extract_struct(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                }
            }

            "enum_item" => {
                if let Some(sym) =
                    symbols::extract_enum(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
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
                    decorators::extract_decorators(&child, source, idx, refs);
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
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                }
            }

            "const_item" => {
                if let Some(sym) =
                    symbols::extract_const(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                }
            }

            "static_item" => {
                if let Some(sym) =
                    symbols::extract_static(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                }
            }

            "mod_item" => {
                if let Some(sym) =
                    symbols::extract_mod(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "use_declaration" => {
                calls::extract_use_names(&child, source, refs, symbols.len());
            }

            // `extern "C" { fn malloc(size: usize) -> *mut u8; }`
            // Walk the declaration_list body and emit Function symbols for each
            // `foreign_item` function declaration.
            "foreign_mod_item" => {
                if let Some(body) = child.child_by_field_name("body") {
                    let mut bc = body.walk();
                    for decl in body.children(&mut bc) {
                        if decl.kind() == "function_item" || decl.kind() == "function_signature_item" {
                            if let Some(sym) =
                                symbols::extract_function(&decl, source, parent_index, qualified_prefix)
                            {
                                let idx = symbols.len();
                                symbols.push(sym);
                                decorators::extract_decorators(&decl, source, idx, refs);
                            }
                        }
                    }
                }
            }

            // `extern crate foo;` — emit an Imports edge for the crate name.
            "extern_crate_declaration" => {
                calls::extract_extern_crate(&child, source, refs, symbols.len());
            }

            // `macro_rules! foo { ... }` — tree-sitter-rust 0.24 emits `macro_definition`.
            // Emit a Function symbol for the macro name.
            "macro_definition" => {
                if let Some(sym) =
                    symbols::extract_macro_rules(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                }
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
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

