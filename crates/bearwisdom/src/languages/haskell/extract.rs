// =============================================================================
// languages/haskell/extract.rs  —  Haskell extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function    — `function` node at top level (or in class/instance body)
//   Struct      — `data_type` / `data_family`
//   Struct      — `newtype`
//   Interface   — `class` (type class)
//   Class       — `instance`
//   TypeAlias   — `type_synomym` / `type_family`
//   Namespace   — `module` header
//
// REFERENCES:
//   Imports     — `import` node
//   Calls       — `apply` node (function application)
//   Implements  — `instance` → type class name
//   Implements  — deriving clause in data_type / newtype
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{
    ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

use super::definitions::{
    extract_data_constructors, extract_deriving, extract_foreign, extract_function,
    extract_import, extract_instance, extract_named_symbol, extract_signature_symbols,
};
use super::expressions::{extract_apply, extract_infix};
use super::servant::extract_servant_routes;

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static HASKELL_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "function",  name_field: "name" },
    ScopeKind { node_kind: "class",     name_field: "name" },
    ScopeKind { node_kind: "data_type", name_field: "name" },
    ScopeKind { node_kind: "newtype",   name_field: "name" },
];

// Haskell built-in type names — skip TypeRef for these.
const BUILTIN_TYPES: &[&str] = &[
    "Int", "Integer", "Float", "Double", "Bool", "Char", "String",
    "IO", "Maybe", "Either", "List", "Ordering", "Word",
    "Int8", "Int16", "Int32", "Int64",
    "Word8", "Word16", "Word32", "Word64",
    "Natural", "Rational", "Complex",
    "()", "[]",
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();

    let mut parser = Parser::new();
    parser.set_language(&lang).expect("Failed to load Haskell grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, HASKELL_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let mut routes: Vec<ExtractedRoute> = Vec::new();

    // Module header (optional)
    extract_module_header(root, src, &mut symbols);

    visit(root, src, &scope_tree, &mut symbols, &mut refs, &mut routes, None, false);

    ExtractionResult::with_connectors(symbols, refs, routes, Vec::<ExtractedDbSet>::new(), has_errors)
}

// ---------------------------------------------------------------------------
// Module header  →  Namespace
// ---------------------------------------------------------------------------

fn extract_module_header(root: Node, src: &[u8], symbols: &mut Vec<ExtractedSymbol>) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "header" {
            // header → module module_id exports? where
            let mut hcursor = child.walk();
            for hchild in child.children(&mut hcursor) {
                if hchild.kind() == "module" {
                    let name = node_text(hchild, src);
                    if !name.is_empty() {
                        symbols.push(make_symbol(
                            name.clone(),
                            name.clone(),
                            SymbolKind::Namespace,
                            &hchild,
                            Some(format!("module {}", name)),
                            None,
                        ));
                    }
                }
            }
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    routes: &mut Vec<ExtractedRoute>,
    parent_index: Option<usize>,
    inside_class_or_instance: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function" => {
                let kind = if inside_class_or_instance {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                let idx = extract_function(&child, src, scope_tree, symbols, kind, parent_index);
                visit(child, src, scope_tree, symbols, refs, routes, idx.or(parent_index), inside_class_or_instance);
            }
            "data_type" | "data_family" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::Struct,
                    "data", parent_index,
                );
                // Extract deriving → Implements
                extract_deriving(&child, src, idx, refs);
                // Extract data_constructor children → EnumMember symbols
                extract_data_constructors(&child, src, idx, symbols);
                visit(child, src, scope_tree, symbols, refs, routes, idx.or(parent_index), false);
            }
            "newtype" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::Struct,
                    "newtype", parent_index,
                );
                extract_deriving(&child, src, idx, refs);
                visit(child, src, scope_tree, symbols, refs, routes, idx.or(parent_index), false);
            }
            "class" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::Interface,
                    "class", parent_index,
                );
                // Recurse into class body — methods inside are Method kind
                visit(child, src, scope_tree, symbols, refs, routes, idx.or(parent_index), true);
            }
            "instance" => {
                let idx = extract_instance(&child, src, scope_tree, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, routes, idx.or(parent_index), true);
            }
            "type_synomym" | "type_family" => {
                let idx = extract_named_symbol(
                    &child, src, scope_tree, symbols, SymbolKind::TypeAlias,
                    "type", parent_index,
                );
                // Servant API type aliases: a chain of `:>` and `:<|>` operators
                // declares HTTP routes. The right-hand side carries the full
                // route surface; emit one ExtractedRoute per Servant verb
                // terminal so each route pairs with Producer-side calls.
                if let Some(handler_idx) = idx {
                    let synonym_src = node_text(child, src);
                    extract_servant_routes(&synonym_src, handler_idx, routes);
                }
            }
            "import" => {
                extract_import(&child, src, symbols, refs, parent_index);
            }
            "apply" => {
                extract_apply(&child, src, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, routes, parent_index, inside_class_or_instance);
            }
            "infix" => {
                extract_infix(&child, src, symbols, refs, parent_index);
                visit(child, src, scope_tree, symbols, refs, routes, parent_index, inside_class_or_instance);
            }
            "foreign_import" | "foreign_export" => {
                extract_foreign(&child, src, scope_tree, symbols, parent_index);
            }
            "signature" => {
                // Type signatures declare callable identifiers. Inside a
                // class body (`class Semigroup a where (<>) :: ...`) they
                // ARE the surface of the typeclass, so emit Method symbols
                // for resolver lookup. At the top level (`foo :: Int`)
                // they declare the type of a binding that may or may not
                // appear elsewhere in the same module — emitting them
                // ensures Prelude operators and forall-style declarations
                // are reachable.
                let kind = if inside_class_or_instance {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                extract_signature_symbols(&child, src, scope_tree, symbols, kind, parent_index);
            }
            _ => {
                visit(child, src, scope_tree, symbols, refs, routes, parent_index, inside_class_or_instance);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    byte_offset: 0,
    }
}

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}
