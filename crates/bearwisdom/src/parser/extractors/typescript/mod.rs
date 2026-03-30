// =============================================================================
// parser/extractors/typescript/mod.rs  —  TypeScript / TSX extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class, Interface, Function (top-level), Method, Constructor, Property,
//   TypeAlias, Variable (const/let/var), Enum, EnumMember
//
// REFERENCES:
//   - `import` statements  → import records (module + named bindings)
//   - `call_expression`    → Calls edges
//   - `extends` / `implements` → Inherits / Implements edges
//   - `fetch(url)` / `axios.{get,post,put,delete}(url)` → candidates for
//     HTTP connector (stored as Calls with target = "fetch" | "axios.get" etc.)
//
// Approach:
//   Same two-pass approach as C#:
//   1. Build scope tree to get qualified names.
//   2. Walk CST to extract symbols and edges.
//
// Note on TSX:
//   TSX files use a slightly different grammar but the symbol node kinds are
//   identical.  The caller passes `is_tsx = true` to select the right grammar.
// =============================================================================

mod calls;
mod helpers;
mod imports;
mod params;
mod symbols;
mod types;

use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration for TypeScript
// ---------------------------------------------------------------------------

static TS_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration", name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
    ScopeKind { node_kind: "method_definition", name_field: "name" },
    // Arrow functions don't have a `name` field — handled separately.
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct TypeScriptExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract symbols and references from TypeScript or TSX source.
pub fn extract(source: &str, is_tsx: bool) -> TypeScriptExtraction {
    let language: tree_sitter::Language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load TypeScript grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return TypeScriptExtraction {
                symbols: vec![],
                refs: vec![],
                has_errors: true,
            }
        }
    };

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();
    let root = tree.root_node();

    let scope_tree = scope_tree::build(root, src_bytes, TS_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src_bytes, &scope_tree, &mut symbols, &mut refs, None);

    TypeScriptExtraction { symbols, refs, has_errors }
}

// ---------------------------------------------------------------------------
// Recursive visitor
// ---------------------------------------------------------------------------

fn extract_node(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                let idx = symbols::push_class(&child, src, scope_tree, symbols, parent_index);
                // Heritage clause (extends / implements).
                imports::extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx =
                    symbols::push_interface(&child, src, scope_tree, symbols, parent_index);
                imports::extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx);
                }
            }

            "function_declaration" => {
                let idx = symbols::push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    types::extract_typed_params_as_symbols(
                        &child,
                        src,
                        scope_tree,
                        symbols,
                        refs,
                        Some(sym_idx),
                    );
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                    }
                }
            }

            "export_statement" => {
                // `export class Foo {}` / `export function bar() {}`
                // Recurse — the declaration itself is a child node.
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            "method_definition" => {
                let idx = symbols::push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Constructor parameter properties:
                    // `constructor(private db: DatabaseRepository)` creates a class property.
                    if symbols[sym_idx].kind == SymbolKind::Constructor {
                        params::extract_constructor_params(
                            &child,
                            src,
                            scope_tree,
                            symbols,
                            refs,
                            parent_index,
                        );
                    }
                    // Parameter types and return type for all methods.
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    // Extract typed params as scoped symbols (for chain resolution).
                    // Skip constructors — they're handled by extract_constructor_params.
                    if symbols[sym_idx].kind != SymbolKind::Constructor {
                        types::extract_typed_params_as_symbols(
                            &child,
                            src,
                            scope_tree,
                            symbols,
                            refs,
                            Some(sym_idx),
                        );
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                    }
                }
            }

            "public_field_definition" | "field_definition" => {
                symbols::push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // Interface property signatures: `db: Database;`
            "property_signature" => {
                symbols::push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // Interface method signatures: `findOne(id: number): T;`
            "method_signature" => {
                let idx = symbols::push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                }
            }

            "type_alias_declaration" => {
                symbols::push_type_alias(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "enum_declaration" => {
                symbols::push_enum(&child, src, scope_tree, symbols, parent_index);
            }

            "lexical_declaration" | "variable_declaration" => {
                // `const Foo = ...` / `let bar = ...`
                symbols::push_variable_decl(&child, src, scope_tree, symbols, refs, parent_index);
            }

            "import_statement" => {
                imports::push_import(&child, src, symbols.len(), refs);
            }

            "for_in_statement" => {
                // for (const item of items) / for (const key in obj)
                // Extract loop variable with chain to iterable for type inference.
                // Then recurse into the body for call extraction.
                params::extract_for_loop_var(
                    &child,
                    src,
                    scope_tree,
                    symbols,
                    refs,
                    parent_index,
                );
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, parent_index);
                }
            }

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
