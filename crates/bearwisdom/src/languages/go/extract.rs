// =============================================================================
// parser/extractors/go/mod.rs  —  Go symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — package name (used as qualified-name prefix)
//   Struct     — `type Foo struct { ... }`
//   Interface  — `type Foo interface { ... }`
//   TypeAlias  — `type Foo Bar` / `type Foo = Bar` (non-struct/interface)
//   Function   — top-level `func Foo(...)`
//   Method     — `func (r ReceiverType) MethodName(...)` → `ReceiverType.MethodName`
//   Method     — interface method element signatures (`method_elem`)
//   Field      — struct fields
//   Variable   — `const` and `var` declarations
//   Test       — functions named Test*, Benchmark*, or Example*
//
// REFERENCES:
//   import_declaration / import_spec → EdgeKind::Imports
//   call_expression                  → EdgeKind::Calls
//   composite_literal                → EdgeKind::Instantiates
//   embedded struct fields           → EdgeKind::Inherits
//
// Visibility:
//   Go has no explicit modifier — exported names start with a Unicode uppercase
//   letter.  Unexported names → Private.
// =============================================================================

use super::{symbols, helpers};
use crate::types::ExtractionResult;
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// Re-exports required by go_tests.rs (`use super::*`).
pub(crate) use crate::types::{EdgeKind, SymbolKind, Visibility};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from Go source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_go::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Go grammar");

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

    let root = tree.root_node();

    // Hoist the package name so it becomes the qualified-name prefix for all
    // top-level symbols.
    let package_name = hoist_package_name(root, source);
    let qualified_prefix = package_name.as_deref().unwrap_or("");

    let mut symbols = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(root, source, &mut symbols, &mut refs, None, qualified_prefix);

    let has_errors = root.has_error();
    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Package name hoist
// ---------------------------------------------------------------------------

/// Find the `package_clause` and return the package identifier text.
///
/// `package_clause` children: `package` (keyword), `package_identifier`.
fn hoist_package_name(root: Node, source: &str) -> Option<String> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_clause" {
            let mut cc = child.walk();
            for inner in child.children(&mut cc) {
                if inner.kind() == "package_identifier" {
                    return Some(helpers::node_text(&inner, source));
                }
            }
        }
    }
    None
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
            // Already handled in hoist pass.
            "package_clause" => {}

            "import_declaration" => {
                symbols::extract_import_declaration(&child, source, refs, symbols.len());
            }

            "function_declaration" => {
                symbols::extract_function_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "method_declaration" => {
                symbols::extract_method_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "type_declaration" => {
                symbols::extract_type_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "const_declaration" => {
                symbols::extract_const_var_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    "const",
                    "const_spec",
                );
            }

            "var_declaration" => {
                symbols::extract_const_var_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    "var",
                    "var_spec",
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(child, source, symbols, refs, parent_index, qualified_prefix);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

