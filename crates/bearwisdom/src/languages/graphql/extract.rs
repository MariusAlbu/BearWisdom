// =============================================================================
// languages/graphql/extract.rs  —  GraphQL SDL/executable extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class         — object_type_definition, union_type_definition
//   Interface     — interface_type_definition
//   Enum          — enum_type_definition
//   EnumMember    — enum_value_definition
//   Struct        — input_object_type_definition
//   TypeAlias     — scalar_type_definition
//   Field         — field_definition, input_value_definition (under parent scope)
//   Function      — directive_definition, operation_definition, fragment_definition
//   Namespace     — schema_definition
//
// REFERENCES:
//   Implements    — implements_interfaces → each named_type
//   TypeRef       — field/input_value type → named_type
//   TypeRef       — union_member_types → named_type
//   TypeRef       — fragment type_condition → named_type
//
// Grammar: tree-sitter-graphql (not yet in Cargo.toml — ready for when added).
// Node names follow the GraphQL SDL grammar conventions.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a GraphQL document.
///
/// Requires the tree-sitter-graphql grammar to be available as `language`.
/// Called by `GraphQlPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load GraphQL grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_document(tree.root_node(), source, &mut symbols, &mut refs);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Document-level traversal
// ---------------------------------------------------------------------------

fn visit_document(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_definition" | "type_system_definition" => {
                // Wrapper node — descend
                visit_document(child, src, symbols, refs);
            }
            "object_type_definition" => extract_object_type(&child, src, symbols, refs),
            "interface_type_definition" => extract_interface_type(&child, src, symbols, refs),
            "enum_type_definition" => extract_enum_type(&child, src, symbols, refs),
            "union_type_definition" => extract_union_type(&child, src, symbols, refs),
            "scalar_type_definition" => extract_scalar_type(&child, src, symbols),
            "input_object_type_definition" => extract_input_type(&child, src, symbols, refs),
            "directive_definition" => extract_directive_def(&child, src, symbols),
            "schema_definition" => extract_schema_def(&child, src, symbols, refs),
            "operation_definition" => extract_operation_def(&child, src, symbols),
            "fragment_definition" => extract_fragment_def(&child, src, symbols, refs),
            // Extensions: emit TypeRef to the extended type
            "object_type_extension"
            | "interface_type_extension"
            | "enum_type_extension"
            | "union_type_extension"
            | "scalar_type_extension"
            | "input_object_type_extension" => {
                extract_type_extension(&child, src, symbols.len(), refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Object type  (type Foo implements Bar { fields })
// ---------------------------------------------------------------------------

fn extract_object_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let doc = extract_description(node, src);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("type {name}")),
        doc_comment: doc,
        scope_path: None,
        parent_index: None,
    });

    // implements_interfaces → Implements edges
    extract_implements(node, src, idx, refs);
    // field_definition children
    extract_fields(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// Interface type
// ---------------------------------------------------------------------------

fn extract_interface_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let doc = extract_description(node, src);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("interface {name}")),
        doc_comment: doc,
        scope_path: None,
        parent_index: None,
    });

    extract_implements(node, src, idx, refs);
    extract_fields(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// Enum type
// ---------------------------------------------------------------------------

fn extract_enum_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let doc = extract_description(node, src);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: doc,
        scope_path: None,
        parent_index: None,
    });

    // enum_value_definition children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_values_definition" {
            extract_enum_values(&child, src, idx, symbols);
        }
    }
}

fn extract_enum_values(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_value_definition" {
            let value_name = match child
                .child_by_field_name("enum_value")
                .or_else(|| first_child_of_kind(&child, "enum_value"))
                .or_else(|| first_child_of_kind(&child, "name"))
                .map(|n| node_text(n, src))
            {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };

            symbols.push(ExtractedSymbol {
                name: value_name.clone(),
                qualified_name: value_name.clone(),
                kind: SymbolKind::EnumMember,
                visibility: Some(Visibility::Public),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(value_name.clone()),
                doc_comment: None,
                scope_path: None,
                parent_index: Some(parent_index),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Union type  (union Result = Ok | Err)
// ---------------------------------------------------------------------------

fn extract_union_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let doc = extract_description(node, src);
    let idx = symbols.len();

    // Build signature from union_member_types
    let members = collect_union_members(node, src);
    let sig = if members.is_empty() {
        format!("union {name}")
    } else {
        format!("union {name} = {}", members.join(" | "))
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment: doc,
        scope_path: None,
        parent_index: None,
    });

    // TypeRef for each member type
    for member in &members {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: member.clone(),
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

fn collect_union_members(node: &Node, src: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "union_member_types" {
            let mut cc = child.walk();
            for named_type in child.children(&mut cc) {
                if named_type.kind() == "named_type" {
                    if let Some(name) = first_child_of_kind(&named_type, "name")
                        .map(|n| node_text(n, src))
                    {
                        if !name.is_empty() {
                            members.push(name);
                        }
                    }
                }
            }
        }
    }
    members
}

// ---------------------------------------------------------------------------
// Scalar type  (scalar DateTime)
// ---------------------------------------------------------------------------

fn extract_scalar_type(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let doc = extract_description(node, src);

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::TypeAlias,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("scalar {name}")),
        doc_comment: doc,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Input object type  (input CreateUserInput { name: String! })
// ---------------------------------------------------------------------------

fn extract_input_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let doc = extract_description(node, src);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Struct,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("input {name}")),
        doc_comment: doc,
        scope_path: None,
        parent_index: None,
    });

    // input_fields_definition → input_value_definition
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "input_fields_definition" {
            extract_input_values(&child, src, idx, symbols, refs);
        }
    }
}

fn extract_input_values(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "input_value_definition" {
            extract_input_value(&child, src, parent_index, symbols, refs);
        }
    }
}

fn extract_input_value(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };

    let type_ref = extract_named_type_from_type_child(node, src);
    let sig = match &type_ref {
        Some(t) => format!("{name}: {t}"),
        None => name.clone(),
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Field,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });

    if let Some(t) = type_ref {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: t,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Directive definition  (directive @deprecated(...) on FIELD_DEFINITION)
// ---------------------------------------------------------------------------

fn extract_directive_def(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("directive @{name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Schema definition  (schema { query: Query })
// ---------------------------------------------------------------------------

fn extract_schema_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: "schema".to_string(),
        qualified_name: "schema".to_string(),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some("schema { ... }".to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // root_operation_type_definition → named_type references
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "root_operation_type_definition" {
            if let Some(type_name) = resolve_named_type_in_subtree(&child, src) {
                refs.push(ExtractedRef {
                    source_symbol_index: idx,
                    target_name: type_name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Operation definition  (query GetUser { ... })
// ---------------------------------------------------------------------------

fn extract_operation_def(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // name is optional — anonymous operations are still valid
    let name = child_name_text(node, src).unwrap_or_else(|| "anonymous".to_string());

    // operation_type: "query" | "mutation" | "subscription"
    let op_type = first_child_of_kind(node, "operation_type")
        .map(|n| node_text(n, src))
        .unwrap_or_else(|| "query".to_string());

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{op_type} {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Fragment definition  (fragment UserFields on User { ... })
// ---------------------------------------------------------------------------

fn extract_fragment_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = if let Some(fn_node) = node.child_by_field_name("fragment_name") {
        first_child_of_kind(&fn_node, "name")
            .map(|n| node_text(n, src))
            .filter(|s| !s.is_empty())
            .or_else(|| child_name_text(node, src))
    } else {
        child_name_text(node, src)
    };

    let name = match name {
        Some(n) => n,
        None => return,
    };

    let idx = symbols.len();

    // type_condition → named_type → name
    let on_type = node
        .child_by_field_name("type_condition")
        .or_else(|| first_child_of_kind(node, "type_condition"))
        .and_then(|tc| resolve_named_type_in_subtree(&tc, src));

    let sig = match &on_type {
        Some(t) => format!("fragment {name} on {t}"),
        None => format!("fragment {name}"),
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    if let Some(t) = on_type {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: t,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Type extension — emit TypeRef to the extended type
// ---------------------------------------------------------------------------

fn extract_type_extension(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(name) = child_name_text(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Shared field extraction
// ---------------------------------------------------------------------------

fn extract_fields(
    parent_node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = parent_node.walk();
    for child in parent_node.children(&mut cursor) {
        if child.kind() == "fields_definition" {
            let mut cc = child.walk();
            for field_node in child.children(&mut cc) {
                if field_node.kind() == "field_definition" {
                    extract_field_def(&field_node, src, parent_index, symbols, refs);
                }
            }
        }
    }
}

fn extract_field_def(
    node: &Node,
    src: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match child_name_text(node, src) {
        Some(n) => n,
        None => return,
    };

    let type_ref = extract_named_type_from_type_child(node, src);
    let sig = match &type_ref {
        Some(t) => format!("{name}: {t}"),
        None => name.clone(),
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Field,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });

    if let Some(t) = type_ref {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: t,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Implements edges
// ---------------------------------------------------------------------------

fn extract_implements(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "implements_interfaces" {
            let mut cc = child.walk();
            for named_type in child.children(&mut cc) {
                if named_type.kind() == "named_type" {
                    if let Some(name) =
                        first_child_of_kind(&named_type, "name").map(|n| node_text(n, src))
                    {
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::Implements,
                                line: named_type.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Type resolution helpers
// ---------------------------------------------------------------------------

/// Get the innermost `named_type` → `name` text from a `type` child node.
/// Handles `non_null_type` and `list_type` wrappers.
fn extract_named_type_from_type_child(node: &Node, src: &str) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;
    resolve_named_type_in_subtree(&type_node, src)
}

/// Recursively unwrap `non_null_type` / `list_type` to reach `named_type`.
fn resolve_named_type_in_subtree(node: &Node, src: &str) -> Option<String> {
    match node.kind() {
        "named_type" => first_child_of_kind(node, "name").map(|n| node_text(n, src)),
        "non_null_type" | "list_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(r) = resolve_named_type_in_subtree(&child, src) {
                    return Some(r);
                }
            }
            None
        }
        _ => {
            // Search children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(r) = resolve_named_type_in_subtree(&child, src) {
                    return Some(r);
                }
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Description (doc comment)
// ---------------------------------------------------------------------------

fn extract_description(node: &Node, src: &str) -> Option<String> {
    let desc_node = node.child_by_field_name("description")
        .or_else(|| first_child_of_kind(node, "description"))?;

    // description contains a string_value child
    let text = first_child_of_kind(&desc_node, "string_value")
        .map(|n| node_text(n, src))
        .unwrap_or_else(|| node_text(desc_node, src));

    let trimmed = text
        .trim_matches('"')
        .trim_matches('`')
        .trim()
        .to_string();

    if trimmed.is_empty() { None } else { Some(trimmed) }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the text of the `name` child of a definition node.
fn child_name_text(node: &Node, src: &str) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| first_child_of_kind(node, "name"))
        .map(|n| node_text(n, src))
        .filter(|s| !s.is_empty())
}

fn first_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
