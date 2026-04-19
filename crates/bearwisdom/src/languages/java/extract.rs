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
                connection_points: Vec::new(),
                demand_contributions: Vec::new(),
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

    // Post-traversal full-tree scan: catch every type_identifier that the main
    // walker missed (e.g. deeply-nested generic type arguments, cast expressions,
    // annotation type names, wildcard bounds, etc.).
    if !symbols.is_empty() {
        scan_all_type_identifiers(root, src_bytes, 0, &mut refs);
    }

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

            // `String value() default "";` inside `@interface` bodies.
            "annotation_type_element_declaration" => {
                symbols::push_annotation_element_decl(&child, src, scope_tree, package, symbols, parent_index);
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
                        // First, walk the method body to extract any nested classes (e.g., anonymous classes).
                        extract_nested_classes_from_body(&body, src, scope_tree, package, symbols, refs, Some(sym_idx));
                        // Then extract calls.
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
                        // First, walk the constructor body to extract any nested classes (e.g., anonymous classes).
                        extract_nested_classes_from_body(&body, src, scope_tree, package, symbols, refs, Some(sym_idx));
                        // Then extract calls.
                        calls::extract_calls_from_body_with_symbols(&body, src, sym_idx, refs, Some(symbols));
                    }
                }
            }

            // Java 16+ compact constructor: `RecordName { ... }` inside record bodies.
            "compact_constructor_declaration" => {
                let idx = symbols::push_compact_constructor_decl(&child, src, scope_tree, package, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    decorators::extract_decorators(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body_with_symbols(&body, src, sym_idx, refs, Some(symbols));
                    }
                }
            }


            "field_declaration" | "constant_declaration" => {
                let field_start_idx = symbols.len();
                symbols::push_field_decl(&child, src, scope_tree, package, symbols, parent_index);
                // Emit annotations on field declarations as TypeRef edges.
                // In tree-sitter-java, annotations on fields appear inside a `modifiers` child.
                let sym_idx = if symbols.len() > field_start_idx {
                    field_start_idx
                } else {
                    parent_index.unwrap_or(0)
                };
                decorators::extract_decorators(&child, src, sym_idx, refs);
                // Emit TypeRef for the field's declared type (including generic args).
                symbols::extract_field_type_refs(&child, src, sym_idx, refs);
                // Extract calls from field/constant initializers, e.g.:
                //   private static final List<X> LIST = buildList();
                //   private final Foo foo = new Foo();
                // tree-sitter: field_declaration → variable_declarator → (field: "value")
                let mut fd_cursor = child.walk();
                for declarator in child.children(&mut fd_cursor) {
                    if declarator.kind() == "variable_declarator" {
                        if let Some(init) = declarator.child_by_field_name("value") {
                            // Extract anonymous class bodies from the initializer.
                            // `Runnable r = new Runnable() { void run() {} };`
                            // The init value may itself BE an object_creation_expression with
                            // an inline class_body, or it may contain one nested deeper.
                            if init.kind() == "object_creation_expression" {
                                // Directly scan for a class_body child.
                                let mut oc = init.walk();
                                for oc_child in init.children(&mut oc) {
                                    if oc_child.kind() == "class_body"
                                        || oc_child.kind() == "anonymous_class_body"
                                    {
                                        extract_node(
                                            oc_child,
                                            src,
                                            scope_tree,
                                            package,
                                            symbols,
                                            refs,
                                            parent_index,
                                        );
                                    }
                                }
                            } else {
                                // Recurse into the init value for nested anonymous classes.
                                extract_nested_classes_from_body(
                                    &init,
                                    src,
                                    scope_tree,
                                    package,
                                    symbols,
                                    refs,
                                    parent_index,
                                );
                            }
                            calls::extract_calls_from_body(&init, src, parent_index.unwrap_or(0), refs);
                        }
                    }
                }
            }

            // `static { ... }` — static initializer block; calls are attributed to
            // the enclosing class (parent_index).
            "static_initializer" => {
                calls::extract_calls_from_body(&child, src, parent_index.unwrap_or(0), refs);
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
// Nested class extraction (from method/constructor bodies)
// ---------------------------------------------------------------------------

/// Walk a method/constructor body and extract any nested classes
/// (including anonymous classes created via `new Type() { ... }`).
fn extract_nested_classes_from_body(
    node: &Node,
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
            // Handle anonymous classes: `new MyInterface() { void method() {} }`
            // The anonymous_class_body directly contains field and method declarations.
            "object_creation_expression" => {
                let mut oc = child.walk();
                for oc_child in child.children(&mut oc) {
                    if oc_child.kind() == "class_body" || oc_child.kind() == "anonymous_class_body" {
                        // Extract from the anonymous class body.
                        extract_node(
                            oc_child,
                            src,
                            scope_tree,
                            package,
                            symbols,
                            refs,
                            parent_index,
                        );
                    } else {
                        // Also scan constructor arguments — they may contain anonymous classes.
                        // e.g. `new Outer(new Inner() { int x; })` — the argument list
                        // contains another object_creation_expression with a class_body.
                        extract_nested_classes_from_body(
                            &oc_child,
                            src,
                            scope_tree,
                            package,
                            symbols,
                            refs,
                            parent_index,
                        );
                    }
                }
            }
            // Recursively walk any other nodes that may contain nested classes.
            _ => {
                extract_nested_classes_from_body(&child, src, scope_tree, package, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type_identifier scanner
// ---------------------------------------------------------------------------

/// Recursively scan ALL descendants of `node` for `type_identifier` nodes and
/// emit a `TypeRef` for each non-primitive name found.
///
/// This is the "nuclear option" post-traversal pass that ensures no type
/// reference is missed regardless of nesting depth (e.g.
/// `List<Map<String, UserDto>>` — finds `UserDto`).
fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::predicates::is_java_builtin;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" && child.is_named() {
            let name = helpers::node_text(child, src);
            if !name.is_empty() && !is_java_builtin(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: crate::types::EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
        // Recurse into ALL children regardless.
        scan_all_type_identifiers(child, src, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

