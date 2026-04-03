// =============================================================================
// parser/extractors/java/mod.rs  —  Java symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace (package), Class, Interface, Enum, EnumMember,
//   Method, Constructor, Field, Test (methods annotated with JUnit/TestNG)
//   Annotation types treated as Interface.
//
// REFERENCES (used to build edges):
//   - `import_declaration`       → Imports edge
//   - `extends` (class)          → Inherits edge
//   - `implements` (class/enum)  → Implements edge
//   - `extends` (interface)      → Implements edge (interface extends interface)
//   - `method_invocation`        → Calls edge
//   - `object_creation_expression` → Instantiates edge
//
// Approach
// --------
// 1. First pass: build a scope tree so we know the qualified name of every
//    position in the file.
// 2. Second pass: walk the CST extracting symbols and references.
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::ExtractionResult;
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration for Java
// ---------------------------------------------------------------------------

pub(crate) static JAVA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",             name_field: "name" },
    ScopeKind { node_kind: "record_declaration",            name_field: "name" },
    ScopeKind { node_kind: "interface_declaration",         name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",              name_field: "name" },
    ScopeKind { node_kind: "annotation_type_declaration",   name_field: "name" },
    ScopeKind { node_kind: "method_declaration",            name_field: "name" },
    ScopeKind { node_kind: "constructor_declaration",       name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// The complete result of extracting one Java file.


/// Parse `source` and extract all symbols and references.
pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Java grammar");

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

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();
    let root = tree.root_node();

    let scope_tree = scope_tree::build(root, src_bytes, JAVA_SCOPE_KINDS);

    // The package name is read once and threaded through qualified name building.
    let package = extract_package(root, src_bytes);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(
        root,
        src_bytes,
        &scope_tree,
        &package,
        &mut symbols,
        &mut refs,
        None,
    );

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Package extraction (first pass, lightweight)
// ---------------------------------------------------------------------------

/// Return the package name declared in the file (e.g. "com.example.service"),
/// or an empty string if there is no package declaration.
fn extract_package(root: Node, src: &[u8]) -> String {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_declaration" {
            // Children: annotation*, identifier | scoped_identifier
            let mut cc = child.walk();
            for c in child.children(&mut cc) {
                match c.kind() {
                    "scoped_identifier" | "identifier" => {
                        return helpers::node_text(c, src);
                    }
                    _ => {}
                }
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn extract_node(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "package_declaration" => {
                symbols::push_package(&child, src, package, symbols, parent_index);
            }

            "import_declaration" => {
                symbols::push_import(&child, src, symbols.len(), refs);
            }

            "class_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Class);
                symbols::extract_class_inheritance(&child, src, idx.unwrap_or(0), refs);
                decorators::extract_decorators(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Interface);
                symbols::extract_interface_inheritance(&child, src, idx.unwrap_or(0), refs);
                decorators::extract_decorators(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "enum_declaration" => {
                let idx = symbols::push_enum_decl(&child, src, scope_tree, package, symbols, parent_index);
                symbols::extract_enum_implements(&child, src, idx.unwrap_or(0), refs);
                decorators::extract_decorators(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    // Extract enum constants first, then recurse for nested declarations.
                    symbols::extract_enum_body(&body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "annotation_type_declaration" => {
                // Treat annotation types as interfaces.
                let idx = symbols::push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Interface);
                decorators::extract_decorators(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            // Java 16+ `record Foo(String name, int age) implements Bar { ... }`
            // Treated as Class — emit symbol + record components as Property symbols.
            "record_declaration" => {
                let idx = symbols::push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Class);
                symbols::extract_class_inheritance(&child, src, idx.unwrap_or(0), refs);
                decorators::extract_decorators(&child, src, idx.unwrap_or(0), refs);
                // Record components (the constructor parameters).
                if let Some(params) = child.child_by_field_name("parameters") {
                    symbols::extract_java_typed_params_as_symbols(&params, src, scope_tree, symbols, refs, idx);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "method_declaration" => {
                let idx = symbols::push_method_decl(&child, src, scope_tree, package, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    decorators::extract_decorators(&child, src, sym_idx, refs);
                    // Extract typed parameters as Property symbols scoped to this method.
                    if let Some(params) = child.child_by_field_name("parameters") {
                        symbols::extract_java_typed_params_as_symbols(&params, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body_with_symbols(&body, src, sym_idx, refs, Some(symbols));
                    }
                }
            }

            "constructor_declaration" => {
                let idx = symbols::push_constructor_decl(&child, src, scope_tree, package, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    decorators::extract_decorators(&child, src, sym_idx, refs);
                    // Extract typed parameters as Property symbols scoped to this constructor.
                    if let Some(params) = child.child_by_field_name("parameters") {
                        symbols::extract_java_typed_params_as_symbols(&params, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body_with_symbols(&body, src, sym_idx, refs, Some(symbols));
                    }
                }
            }

            "field_declaration" | "constant_declaration" => {
                symbols::push_field_decl(&child, src, scope_tree, package, symbols, parent_index);
            }

            "ERROR" | "MISSING" => {
                // tree-sitter error recovery — skip.
            }

            _ => {
                extract_node(child, src, scope_tree, package, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

