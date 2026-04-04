// =============================================================================
// java/symbols.rs  —  Symbol pushers for Java declarations
// =============================================================================

use super::helpers::{
    build_method_signature, detect_visibility, extract_doc_comment, find_formal_param_name,
    has_test_annotation, is_java_primitive, java_type_node_simple_name, node_text,
    qualify_with_package, scope_path_with_package, type_node_simple_name,
};
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn push_package(
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

pub(super) fn push_type_decl(
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

pub(super) fn push_enum_decl(
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

pub(super) fn extract_enum_body(
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
                super::extract::extract_node(child, src, scope_tree, package, symbols, refs, enum_parent_index);
            }
        }
    }
}

pub(super) fn push_method_decl(
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

pub(super) fn push_constructor_decl(
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

pub(super) fn push_field_decl(
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

    // The field_declaration node start line — used for coverage correlation.
    // Annotated fields like `@Column\nprivate String name;` have the annotation
    // on a higher line than the variable_declarator. The coverage tool correlates
    // by (field_declaration, field_decl_start_line), so we must emit at that line.
    let field_decl_start_line = node.start_position().row as u32;

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
                    // Use the field_declaration start line (may include leading annotations)
                    // rather than the variable_declarator line, so coverage correlation
                    // matches correctly for annotated fields.
                    start_line: field_decl_start_line,
                    end_line: child.end_position().row as u32,
                    start_col: node.start_position().column as u32,
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

/// Emit TypeRef edges for a `field_declaration` or `constant_declaration`'s
/// declared type, including generic type arguments.
///
/// For `private List<User> users;` → TypeRef to `List` and TypeRef to `User`.
pub(super) fn extract_field_type_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_recursive(type_node, src, source_symbol_index, refs);
    }
}

/// Recursively extract TypeRef edges from a Java type node.
///
/// Handles: `type_identifier`, `generic_type`, `scoped_type_identifier`,
/// `array_type`, `annotated_type`, `wildcard_type`, `type_arguments`.
pub(super) fn extract_type_refs_recursive(
    type_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match type_node.kind() {
        "type_identifier" => {
            let name = node_text(type_node, src);
            if !name.is_empty() && !is_java_primitive(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "generic_type" => {
            // e.g. `List<User>` → emit TypeRef for "List" and recurse into type_arguments.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" | "scoped_type_identifier" => {
                        let name = type_node_simple_name(child, src);
                        if !name.is_empty() && !is_java_primitive(&name) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    "type_arguments" => {
                        extract_type_refs_recursive(child, src, source_symbol_index, refs);
                    }
                    _ => {}
                }
            }
        }
        "type_arguments" => {
            // e.g. `<String, Integer>` — recurse into each argument.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_recursive(child, src, source_symbol_index, refs);
                }
            }
        }
        "scoped_type_identifier" => {
            let name = type_node_simple_name(type_node, src);
            if !name.is_empty() && !is_java_primitive(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "array_type" => {
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_recursive(child, src, source_symbol_index, refs);
                    break;
                }
            }
        }
        "annotated_type" | "wildcard" => {
            // Recurse into inner type.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_recursive(child, src, source_symbol_index, refs);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
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
                    chain: None,
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
                    chain: None,
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
pub(super) fn extract_class_inheritance(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `superclass` is a named field on class_declaration.
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
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
                    chain: None,
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
pub(super) fn extract_interface_inheritance(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_interfaces" {
            extract_type_list_as_implements(child, src, source_idx, refs);
        }
    }
}

/// Extract `implements I1, I2` from an enum declaration.
pub(super) fn extract_enum_implements(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
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
                        chain: None,
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Typed parameter symbol extraction
// ---------------------------------------------------------------------------

/// Extract typed parameters from a Java `formal_parameters` node as Property
/// symbols scoped to the enclosing method or constructor.
pub(super) fn extract_java_typed_params_as_symbols(
    params_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let method_scope = if params_node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, params_node.start_byte())
    } else {
        None
    };

    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() != "formal_parameter" && child.kind() != "spread_parameter" {
            continue;
        }

        let type_node = match child.child_by_field_name("type") {
            Some(t) => t,
            None => continue,
        };

        let name = find_formal_param_name(&child, src);
        if name.is_empty() {
            continue;
        }

        let type_name = java_type_node_simple_name(type_node, src);
        if type_name.is_empty() || is_java_primitive(&type_name) {
            continue;
        }

        let qualified_name = scope_tree::qualify(&name, method_scope);
        let scope_path = scope_tree::scope_path(method_scope);

        let param_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: None,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: Some(format!("{type_name} {name}")),
            doc_comment: None,
            scope_path,
            parent_index,
        });

        refs.push(ExtractedRef {
            source_symbol_index: param_idx,
            target_name: type_name,
            kind: EdgeKind::TypeRef,
            line: type_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}
