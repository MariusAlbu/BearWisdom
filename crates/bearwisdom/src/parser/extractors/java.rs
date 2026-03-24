// =============================================================================
// parser/extractors/java.rs  —  Java symbol and reference extractor
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
// ANNOTATIONS:
//   @Test, @ParameterizedTest, @RepeatedTest (JUnit 5)
//   @Test (JUnit 4 / TestNG)
//   → promote the enclosing method to SymbolKind::Test
//
// Approach
// --------
// 1. First pass: build a scope tree so we know the qualified name of every
//    position in the file.
// 2. Second pass: walk the CST extracting symbols and references.
//
// Grammar notes (tree-sitter-java 0.23.5):
//   - `modifiers` is an unnamed child of declarations; visibility keywords
//     (public/private/protected) are unnamed leaf tokens inside `modifiers`.
//   - `method_invocation` exposes `.name` (identifier) and optional `.object`.
//   - `object_creation_expression` exposes `.type` (_simple_type).
//   - `superclass` (child of class_declaration) has a single unnamed `_type` child.
//   - `super_interfaces` / `extends_interfaces` both contain a `type_list`.
//   - `import_declaration` has no named fields; children are identifier /
//     scoped_identifier / asterisk.
//   - `package_declaration` has no named fields; children include scoped_identifier.
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration for Java
// ---------------------------------------------------------------------------

static JAVA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",             name_field: "name" },
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
pub struct JavaExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Parse `source` and extract all symbols and references.
pub fn extract(source: &str) -> JavaExtraction {
    let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Java grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return JavaExtraction {
                symbols: vec![],
                refs: vec![],
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

    JavaExtraction { symbols, refs, has_errors }
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
                        return node_text(c, src);
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
fn extract_node(
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
                push_package(&child, src, package, symbols, parent_index);
            }

            "import_declaration" => {
                push_import(&child, src, symbols.len(), refs);
            }

            "class_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Class);
                extract_class_inheritance(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "interface_declaration" => {
                let idx = push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Interface);
                extract_interface_inheritance(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "enum_declaration" => {
                let idx = push_enum_decl(&child, src, scope_tree, package, symbols, parent_index);
                extract_enum_implements(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    // Extract enum constants first, then recurse for nested declarations.
                    extract_enum_body(&body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "annotation_type_declaration" => {
                // Treat annotation types as interfaces.
                let idx = push_type_decl(&child, src, scope_tree, package, symbols, parent_index, SymbolKind::Interface);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, package, symbols, refs, idx);
                }
            }

            "method_declaration" => {
                let idx = push_method_decl(&child, src, scope_tree, package, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "constructor_declaration" => {
                let idx = push_constructor_decl(&child, src, scope_tree, package, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "field_declaration" | "constant_declaration" => {
                push_field_decl(&child, src, scope_tree, package, symbols, parent_index);
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
// Symbol pushers
// ---------------------------------------------------------------------------

fn push_package(
    node: &Node,
    _src: &[u8],
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    if package.is_empty() {
        return None;
    }
    // Simple name: last segment of the dotted package path.
    let name = package.rsplit('.').next().unwrap_or(package).to_string();
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name: package.to_string(),
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("package {package}")),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
    Some(idx)
}

fn push_type_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    kind: SymbolKind,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = qualify_with_package(&name, parent_scope, package);
    let scope_path = scope_path_with_package(parent_scope, package);

    let keyword = match kind {
        SymbolKind::Interface => "interface",
        SymbolKind::Enum => "enum",
        _ => "class",
    };
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{keyword} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_enum_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = qualify_with_package(&name, parent_scope, package);
    let scope_path = scope_path_with_package(parent_scope, package);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Enum,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn extract_enum_body(
    body: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    enum_parent_index: Option<usize>,
) {
    // Qualified name of the enum itself — needed to prefix constant names.
    let enum_qname = enum_parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "enum_constant" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, src);
                    let qualified_name = if enum_qname.is_empty() {
                        name.clone()
                    } else {
                        format!("{enum_qname}.{name}")
                    };
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name,
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: None,
                        doc_comment: extract_doc_comment(&child, src),
                        scope_path: if enum_qname.is_empty() { None } else { Some(enum_qname.clone()) },
                        parent_index: enum_parent_index,
                    });
                }
            }
            // Enum body can also contain class_body declarations.
            _ => {
                extract_node(child, src, scope_tree, package, symbols, refs, enum_parent_index);
            }
        }
    }
}

fn push_method_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = qualify_with_package(&name, parent_scope, package);
    let scope_path = scope_path_with_package(parent_scope, package);

    let kind = if has_test_annotation(node, src) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: build_method_signature(node, src),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_constructor_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = qualify_with_package(&name, parent_scope, package);
    let scope_path = scope_path_with_package(parent_scope, package);

    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

fn push_field_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    package: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Both field_declaration and constant_declaration use a `type` field
    // and one or more `declarator` children (variable_declarator).
    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let visibility = detect_visibility(node, src);
    let doc_comment = extract_doc_comment(node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_path_with_package(parent_scope, package);

    // Iterate over the declarator children by kind (grammar: field="declarator").
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = qualify_with_package(&name, parent_scope, package);
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name,
                    kind: SymbolKind::Field,
                    visibility,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: Some(format!("{type_str} {name}")),
                    doc_comment: doc_comment.clone(),
                    scope_path: scope_path.clone(),
                    parent_index,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Grammar: children are identifier | scoped_identifier | asterisk.
    // For `import java.util.List` → scoped_identifier → full path "java.util.List"
    // For `import java.util.*`    → scoped_identifier + asterisk
    // For `import static ...`     → same structure with a `static` keyword token.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" => {
                let full = node_text(child, src);
                // imported_name = simple name (last segment)
                let imported = full.rsplit('.').next().unwrap_or(&full).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: imported,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                });
                return;
            }
            "identifier" => {
                let name = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(name),
                });
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Inheritance / implementation edges
// ---------------------------------------------------------------------------

/// Extract `extends BaseClass` and `implements I1, I2` from a class declaration.
fn extract_class_inheritance(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `superclass` is a named field on class_declaration.
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        // superclass node has a single unnamed child of type `_type`.
        let mut cursor = superclass_node.walk();
        for child in superclass_node.children(&mut cursor) {
            let name = type_node_simple_name(child, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Inherits,
                    line: child.start_position().row as u32,
                    module: None,
                });
                break;
            }
        }
    }

    // `interfaces` is a named field that points to a `super_interfaces` node,
    // which contains a `type_list`.
    if let Some(ifaces_node) = node.child_by_field_name("interfaces") {
        extract_type_list_as_implements(ifaces_node, src, source_idx, refs);
    }
}

/// Extract `extends I1, I2` from an interface declaration.
/// In Java, interface extends means "extends" which we treat as Implements
/// (since interface→interface extension carries the same semantic as class→interface).
fn extract_interface_inheritance(node: &Node, src: &[u8], source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    // `extends_interfaces` is an unnamed child of interface_declaration.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_interfaces" {
            extract_type_list_as_implements(child, src, source_idx, refs);
        }
    }
}

/// Extract `implements I1, I2` from an enum declaration.
fn extract_enum_implements(node: &Node, src: &[u8], source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    if let Some(ifaces_node) = node.child_by_field_name("interfaces") {
        extract_type_list_as_implements(ifaces_node, src, source_idx, refs);
    }
}

/// Walk a `super_interfaces`, `extends_interfaces`, or any wrapper that contains
/// a `type_list`, and emit one `Implements` ref per named type.
fn extract_type_list_as_implements(
    container: Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // container is super_interfaces or extends_interfaces — both hold a type_list.
    let mut outer = container.walk();
    for child in container.children(&mut outer) {
        if child.kind() == "type_list" {
            let mut cursor = child.walk();
            for type_node in child.children(&mut cursor) {
                let name = type_node_simple_name(type_node, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Implements,
                        line: type_node.start_position().row as u32,
                        module: None,
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Call / instantiation extraction
// ---------------------------------------------------------------------------

fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_invocation" => {
                // `name` field is always present (identifier).
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: name_node.start_position().row as u32,
                            module: None,
                        });
                    }
                }
                // Recurse into arguments — nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = type_node_simple_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers — annotations
// ---------------------------------------------------------------------------

const TEST_ANNOTATIONS: &[&str] = &[
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "TestFactory",
    "TestTemplate",
];

/// Returns true if any `marker_annotation` or `annotation` in the `modifiers`
/// (or as a direct child of the method node) is a JUnit/TestNG test annotation.
fn has_test_annotation(node: &Node, src: &[u8]) -> bool {
    // Annotations can appear inside `modifiers` or directly as unnamed children.
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        match child.kind() {
            "modifiers" => {
                let mut mc = child.walk();
                for ann in child.children(&mut mc) {
                    if annotation_is_test(ann, src) {
                        return true;
                    }
                }
            }
            "marker_annotation" | "annotation" => {
                if annotation_is_test(child, src) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn annotation_is_test(node: Node, src: &[u8]) -> bool {
    if node.kind() != "marker_annotation" && node.kind() != "annotation" {
        return false;
    }
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(name_node, src);
        return TEST_ANNOTATIONS.contains(&name.as_str());
    }
    false
}

// ---------------------------------------------------------------------------
// Helpers — visibility
// ---------------------------------------------------------------------------

/// Detect visibility from the `modifiers` child of a declaration node.
///
/// In tree-sitter-java, `modifiers` is an unnamed child containing unnamed
/// leaf tokens like "public", "private", "protected".
fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mod_text = node_text(child, src);
            // Fast path: scan the modifier text rather than iterating unnamed tokens.
            // This avoids needing child access on unnamed token nodes.
            if mod_text.contains("public") {
                return Some(Visibility::Public);
            }
            if mod_text.contains("private") {
                return Some(Visibility::Private);
            }
            if mod_text.contains("protected") {
                return Some(Visibility::Protected);
            }
            // No visibility keyword → package-private.
            return None;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers — doc comments (Javadoc)
// ---------------------------------------------------------------------------

/// Extract a Javadoc comment (`/** ... */`) immediately preceding `node`.
fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        let text = node_text(s, src);
        let trimmed = text.trim_start();
        if trimmed.starts_with("/**") {
            return Some(text);
        }
        // Skip plain block comments and whitespace-only siblings.
        if trimmed.starts_with("/*") || trimmed.is_empty() {
            sib = s.prev_sibling();
            continue;
        }
        break;
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers — method signatures
// ---------------------------------------------------------------------------

fn build_method_signature(node: &Node, src: &[u8]) -> Option<String> {
    let name = node_text(node.child_by_field_name("name")?, src);
    let ret = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| format_params(p, src))
        .unwrap_or_default();
    let sig = format!("{ret} {type_params}{name}{params}").trim().to_string();
    Some(sig)
}

/// Build a compact parameter list string: `(String name, int id)`.
fn format_params(params_node: Node, src: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "formal_parameter" || child.kind() == "spread_parameter" {
            let type_str = child
                .child_by_field_name("type")
                .map(|t| node_text(t, src))
                .unwrap_or_default();
            let name_str = child
                .child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            if type_str.is_empty() {
                parts.push(name_str);
            } else {
                parts.push(format!("{type_str} {name_str}"));
            }
        }
    }
    format!("({})", parts.join(", "))
}

// ---------------------------------------------------------------------------
// Helpers — qualified names
// ---------------------------------------------------------------------------

/// Build a qualified name by combining the parent scope path with `name`,
/// then prepending the package if the scope path doesn't already start with it.
fn qualify_with_package(
    name: &str,
    parent_scope: Option<&scope_tree::ScopeEntry>,
    package: &str,
) -> String {
    match parent_scope {
        Some(scope) => {
            // Scope already carries the full qualified name up to the parent.
            // If we're in a package and the scope doesn't start with the package,
            // prepend it.
            let base = &scope.qualified_name;
            if !package.is_empty() && !base.starts_with(package) {
                format!("{package}.{base}.{name}")
            } else {
                format!("{base}.{name}")
            }
        }
        None => {
            if package.is_empty() {
                name.to_string()
            } else {
                format!("{package}.{name}")
            }
        }
    }
}

/// Build the scope_path string: the parent's qualified name, prefixed with
/// the package if needed.
fn scope_path_with_package(
    parent_scope: Option<&scope_tree::ScopeEntry>,
    package: &str,
) -> Option<String> {
    match parent_scope {
        Some(scope) => {
            let base = &scope.qualified_name;
            if !package.is_empty() && !base.starts_with(package) {
                Some(format!("{package}.{base}"))
            } else {
                Some(base.clone())
            }
        }
        None => {
            if package.is_empty() {
                None
            } else {
                Some(package.to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers — type name extraction
// ---------------------------------------------------------------------------

/// Extract the simple (unqualified) name from a type node.
///
/// Handles:
/// - `type_identifier`       → raw text (e.g. "List")
/// - `generic_type`          → first type_identifier child (e.g. "List" from "List<User>")
/// - `scoped_type_identifier` → last segment (e.g. "UserService" from "com.example.UserService")
/// - `array_type`            → recurse into element type
fn type_node_simple_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, src),
        "generic_type" => {
            // Children: type_identifier | scoped_type_identifier, type_arguments
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" => return node_text(child, src),
                    "scoped_type_identifier" => {
                        let full = node_text(child, src);
                        return full.rsplit('.').next().unwrap_or(&full).to_string();
                    }
                    _ => {}
                }
            }
            String::new()
        }
        "scoped_type_identifier" => {
            let full = node_text(node, src);
            full.rsplit('.').next().unwrap_or(&full).to_string()
        }
        "array_type" => {
            // element type is the first _unannotated_type child.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let name = type_node_simple_name(child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "annotated_type" => {
            // Strip annotations and recurse into the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "annotation" | "marker_annotation" => continue,
                    _ => {
                        let name = type_node_simple_name(child, src);
                        if !name.is_empty() {
                            return name;
                        }
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    fn sym(source: &str) -> Vec<ExtractedSymbol> {
        extract(source).symbols
    }
    fn refs(source: &str) -> Vec<ExtractedRef> {
        extract(source).refs
    }

    // -----------------------------------------------------------------------
    // 1. Class with methods and fields
    // -----------------------------------------------------------------------
    #[test]
    fn class_with_methods_and_fields() {
        let src = r#"
package com.example;

public class UserService {
    private String name;

    public String getName() { return name; }

    protected void setName(String name) { this.name = name; }
}
"#;
        let symbols = sym(src);

        let cls = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(cls.kind, SymbolKind::Class);
        assert_eq!(cls.visibility, Some(Visibility::Public));
        assert_eq!(cls.qualified_name, "com.example.UserService");

        let field = symbols.iter().find(|s| s.name == "name" && s.kind == SymbolKind::Field).unwrap();
        assert_eq!(field.visibility, Some(Visibility::Private));
        assert!(field.signature.as_ref().unwrap().contains("String"));

        let get_name = symbols.iter().find(|s| s.name == "getName").unwrap();
        assert_eq!(get_name.kind, SymbolKind::Method);
        assert_eq!(get_name.visibility, Some(Visibility::Public));
        assert_eq!(get_name.qualified_name, "com.example.UserService.getName");

        let set_name = symbols.iter().find(|s| s.name == "setName").unwrap();
        assert_eq!(set_name.visibility, Some(Visibility::Protected));
    }

    // -----------------------------------------------------------------------
    // 2. Interface with default methods
    // -----------------------------------------------------------------------
    #[test]
    fn interface_with_default_method() {
        let src = r#"
package com.example;

public interface Repository<T> {
    T findById(long id);

    default void delete(long id) {}
}
"#;
        let symbols = sym(src);
        let iface = symbols.iter().find(|s| s.name == "Repository").unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.qualified_name, "com.example.Repository");

        assert!(symbols.iter().any(|s| s.name == "findById" && s.kind == SymbolKind::Method));
        assert!(symbols.iter().any(|s| s.name == "delete"    && s.kind == SymbolKind::Method));
    }

    // -----------------------------------------------------------------------
    // 3. Enum with constants
    // -----------------------------------------------------------------------
    #[test]
    fn enum_with_constants() {
        let src = r#"
package com.example;

public enum Status {
    PENDING,
    ACTIVE,
    DELETED;

    public boolean isActive() { return this == ACTIVE; }
}
"#;
        let symbols = sym(src);

        let e = symbols.iter().find(|s| s.name == "Status" && s.kind == SymbolKind::Enum).unwrap();
        assert_eq!(e.qualified_name, "com.example.Status");

        let members: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::EnumMember).collect();
        let names: Vec<&str> = members.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"PENDING"), "got: {names:?}");
        assert!(names.contains(&"ACTIVE"),  "got: {names:?}");
        assert!(names.contains(&"DELETED"), "got: {names:?}");

        // Method inside enum body.
        assert!(symbols.iter().any(|s| s.name == "isActive" && s.kind == SymbolKind::Method));
    }

    // -----------------------------------------------------------------------
    // 4. Nested / inner classes get qualified names
    // -----------------------------------------------------------------------
    #[test]
    fn nested_class_qualified_name() {
        let src = r#"
package com.example;

public class Outer {
    public static class Inner {
        public void work() {}
    }
}
"#;
        let symbols = sym(src);

        let outer = symbols.iter().find(|s| s.name == "Outer").unwrap();
        assert_eq!(outer.qualified_name, "com.example.Outer");

        let inner = symbols.iter().find(|s| s.name == "Inner").unwrap();
        // Inner should be qualified relative to Outer.
        assert!(
            inner.qualified_name.contains("Outer.Inner"),
            "expected Outer.Inner in qualified_name, got: {}",
            inner.qualified_name
        );

        let method = symbols.iter().find(|s| s.name == "work").unwrap();
        assert!(
            method.qualified_name.contains("Inner.work"),
            "expected Inner.work in qualified_name, got: {}",
            method.qualified_name
        );
    }

    // -----------------------------------------------------------------------
    // 5. Imports extracted correctly
    // -----------------------------------------------------------------------
    #[test]
    fn import_extracted() {
        let src = r#"
import java.util.List;
import java.util.ArrayList;
import static org.junit.Assert.*;
"#;
        let r = refs(src);
        let imports: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Imports).collect();

        let list_import = imports.iter().find(|i| i.target_name == "List");
        assert!(list_import.is_some(), "Missing List import; imports: {:?}",
            imports.iter().map(|i| &i.target_name).collect::<Vec<_>>());
        assert_eq!(list_import.unwrap().module.as_deref(), Some("java.util.List"));

        assert!(imports.iter().any(|i| i.target_name == "ArrayList"));
    }

    // -----------------------------------------------------------------------
    // 6. Method calls create Calls edges
    // -----------------------------------------------------------------------
    #[test]
    fn method_calls_create_edges() {
        let src = r#"
class Service {
    void run() {
        foo();
        bar.baz();
        String s = helper.doSomething();
    }
}
"#;
        let r = refs(src);
        let calls: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let names: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();

        assert!(names.contains(&"foo"),         "Missing foo; calls: {names:?}");
        assert!(names.contains(&"baz"),         "Missing baz; calls: {names:?}");
        assert!(names.contains(&"doSomething"), "Missing doSomething; calls: {names:?}");
    }

    // -----------------------------------------------------------------------
    // 7. @Test annotation promotes method to SymbolKind::Test
    // -----------------------------------------------------------------------
    #[test]
    fn test_annotation_detected() {
        let src = r#"
import org.junit.jupiter.api.Test;

class CalculatorTest {
    @Test
    void addsTwoNumbers() {
        assert 1 + 1 == 2;
    }

    @ParameterizedTest
    void addsManyNumbers(int a, int b) {}

    void helperMethod() {}
}
"#;
        let symbols = sym(src);

        let adds = symbols.iter().find(|s| s.name == "addsTwoNumbers").unwrap();
        assert_eq!(adds.kind, SymbolKind::Test, "addsTwoNumbers should be Test");

        let parameterized = symbols.iter().find(|s| s.name == "addsManyNumbers").unwrap();
        assert_eq!(parameterized.kind, SymbolKind::Test, "addsManyNumbers should be Test");

        let helper = symbols.iter().find(|s| s.name == "helperMethod").unwrap();
        assert_eq!(helper.kind, SymbolKind::Method, "helperMethod should be Method");
    }

    // -----------------------------------------------------------------------
    // 8. Inheritance and implementation create correct edge kinds
    // -----------------------------------------------------------------------
    #[test]
    fn inheritance_edges() {
        let src = r#"
package com.example;

public class UserService extends BaseService implements Serializable, Cloneable {}
"#;
        let r = refs(src);

        assert!(
            r.iter().any(|r| r.target_name == "BaseService" && r.kind == EdgeKind::Inherits),
            "Expected Inherits edge to BaseService; refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "Serializable" && r.kind == EdgeKind::Implements),
            "Expected Implements edge to Serializable"
        );
        assert!(
            r.iter().any(|r| r.target_name == "Cloneable" && r.kind == EdgeKind::Implements),
            "Expected Implements edge to Cloneable"
        );
    }

    // -----------------------------------------------------------------------
    // 9. Interface extends interface → Implements edges
    // -----------------------------------------------------------------------
    #[test]
    fn interface_extends_interface() {
        let src = r#"
public interface ExtendedRepo extends Repository, ReadOnly {}
"#;
        let r = refs(src);

        assert!(
            r.iter().any(|r| r.target_name == "Repository" && r.kind == EdgeKind::Implements),
            "Expected Implements for Repository"
        );
        assert!(
            r.iter().any(|r| r.target_name == "ReadOnly" && r.kind == EdgeKind::Implements),
            "Expected Implements for ReadOnly"
        );
    }

    // -----------------------------------------------------------------------
    // 10. Visibility extraction
    // -----------------------------------------------------------------------
    #[test]
    fn visibility_extraction() {
        let src = r#"
class Example {
    public int pub;
    private int priv;
    protected int prot;
    int packagePrivate;
}
"#;
        let symbols = sym(src);

        let pub_field = symbols.iter().find(|s| s.name == "pub").unwrap();
        assert_eq!(pub_field.visibility, Some(Visibility::Public));

        let priv_field = symbols.iter().find(|s| s.name == "priv").unwrap();
        assert_eq!(priv_field.visibility, Some(Visibility::Private));

        let prot_field = symbols.iter().find(|s| s.name == "prot").unwrap();
        assert_eq!(prot_field.visibility, Some(Visibility::Protected));

        let pkg_field = symbols.iter().find(|s| s.name == "packagePrivate").unwrap();
        assert_eq!(pkg_field.visibility, None, "package-private should be None");
    }

    // -----------------------------------------------------------------------
    // 11. object_creation_expression → Instantiates edge
    // -----------------------------------------------------------------------
    #[test]
    fn instantiation_edges() {
        let src = r#"
class Factory {
    void create() {
        Foo f = new Foo();
        List<Bar> bars = new ArrayList<>();
    }
}
"#;
        let r = refs(src);
        let instantiations: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Instantiates).collect();
        let names: Vec<&str> = instantiations.iter().map(|r| r.target_name.as_str()).collect();

        assert!(names.contains(&"Foo"),       "Missing Foo; got: {names:?}");
        assert!(names.contains(&"ArrayList"), "Missing ArrayList; got: {names:?}");
    }

    // -----------------------------------------------------------------------
    // 12. Constructor extraction
    // -----------------------------------------------------------------------
    #[test]
    fn constructor_extracted() {
        let src = r#"
class Svc {
    public Svc(String name, int port) {}
}
"#;
        let symbols = sym(src);
        let ctor = symbols.iter().find(|s| s.kind == SymbolKind::Constructor).unwrap();
        assert_eq!(ctor.name, "Svc");
        assert_eq!(ctor.visibility, Some(Visibility::Public));
        let sig = ctor.signature.as_ref().unwrap();
        assert!(sig.contains("String"), "signature: {sig}");
        assert!(sig.contains("int"),    "signature: {sig}");
    }

    // -----------------------------------------------------------------------
    // 13. Does not panic on malformed source
    // -----------------------------------------------------------------------
    #[test]
    fn does_not_panic_on_malformed_source() {
        let src = "public class { broken !!! @@@ ###";
        let _ = extract(src); // must not panic
    }

    // -----------------------------------------------------------------------
    // 14. Annotation type treated as interface
    // -----------------------------------------------------------------------
    #[test]
    fn annotation_type_as_interface() {
        let src = r#"
package com.example;

public @interface MyAnnotation {
    String value() default "";
}
"#;
        let symbols = sym(src);
        let ann = symbols.iter().find(|s| s.name == "MyAnnotation").unwrap();
        assert_eq!(ann.kind, SymbolKind::Interface);
        assert_eq!(ann.qualified_name, "com.example.MyAnnotation");
    }
}
