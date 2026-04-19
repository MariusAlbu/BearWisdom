// =============================================================================
// kotlin/symbols.rs  —  Symbol pushers and import/delegation helpers for Kotlin
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, find_child_by_kind, node_text,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Class body dispatchers
// ---------------------------------------------------------------------------

pub(super) fn extract_class_body(
    class_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        match child.kind() {
            "class_body" => {
                super::extract::extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
            "enum_class_body" => {
                extract_enum_class_body(&child, src, scope_tree, symbols, refs, parent_index);
            }
            _ => {}
        }
    }
}

pub(super) fn extract_enum_class_body(
    body: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "enum_entry" => {
                // tree-sitter-kotlin-ng may use a `name` field or a direct
                // `simple_identifier` child — handle both.
                let name_opt = child.child_by_field_name("name")
                    .map(|n| node_text(n, src))
                    .or_else(|| {
                        let mut cc = child.walk();
                        for inner in child.children(&mut cc) {
                            if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                                let t = node_text(inner, src);
                                if !t.is_empty() {
                                    return Some(t);
                                }
                            }
                        }
                        None
                    });
                if let Some(name) = name_opt {
                    let qualified_name = if enum_qname.is_empty() {
                        name.clone()
                    } else {
                        format!("{enum_qname}.{name}")
                    };
                    symbols.push(ExtractedSymbol {
                        name,
                        qualified_name,
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: if enum_qname.is_empty() { None } else { Some(enum_qname.clone()) },
                        parent_index,
                    });
                }
            }
            _ => {
                super::extract::extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

pub(super) fn push_type_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = match kind {
        SymbolKind::Class     => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum      => "enum class",
        _                     => "class",
    };

    let visibility = detect_visibility(node, src);
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_function_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if scope.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let params = node
        .child_by_field_name("function_value_parameters")
        .or_else(|| find_child_by_kind(node, "value_arguments"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("type")
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();
    let signature = Some(format!("fun {name}{params}{ret}"));

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
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_property_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // In tree-sitter-kotlin-ng, `property_declaration` has no `name` field.
    // The identifier is inside a `variable_declaration` child.
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declaration" {
                    let mut cc = child.walk();
                    for inner in child.children(&mut cc) {
                        if inner.kind() == "identifier" || inner.kind() == "simple_identifier" {
                            return Some(node_text(inner, src));
                        }
                    }
                }
            }
            None
        });
    let name = match name {
        Some(n) => n,
        None    => return,
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = if node_text(*node, src).trim_start().starts_with("val") { "val" } else { "var" };
    let ty = node
        .child_by_field_name("type")
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}{ty}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

/// Emit a Class symbol for a `companion object [Name]` declaration.
/// Returns the symbol index.
pub(super) fn push_companion_object(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // Name is optional — default to "Companion" per Kotlin spec.
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or_else(|| "Companion".to_string());

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Class,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some("companion object".to_string()),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract Variable symbols (and TypeRef edges) from a `primary_constructor`'s
/// `class_parameters`. Parameters with `val`/`var` modifiers also become
/// Property symbols (Kotlin primary-constructor promotion).
/// Also emits a Constructor symbol for the primary constructor itself.
pub(super) fn extract_primary_constructor_params(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // `primary_constructor` is a non-field child of `class_declaration`.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "primary_constructor" {
            // Emit a Constructor symbol for the primary constructor.
            let class_name = parent_index
                .and_then(|i| symbols.get(i))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "constructor".to_string());

            let scope = enclosing_scope(scope_tree, child.start_byte(), child.end_byte());
            let qualified_name = scope_tree::qualify(&class_name, scope);
            let scope_path = scope_tree::scope_path(scope);

            let params_text = find_child_by_kind(&child, "class_parameters")
                .map(|p| node_text(p, src))
                .unwrap_or_default();

            symbols.push(ExtractedSymbol {
                name: class_name.clone(),
                qualified_name,
                kind: SymbolKind::Constructor,
                visibility: detect_visibility(&child, src),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(format!("{class_name}{params_text}")),
                doc_comment: extract_doc_comment(&child, src),
                scope_path,
                parent_index,
            });

            let mut pc = child.walk();
            for inner in child.children(&mut pc) {
                if inner.kind() == "class_parameters" {
                    let mut cc = inner.walk();
                    for param in inner.children(&mut cc) {
                        if param.kind() == "class_parameter" {
                            extract_class_parameter(
                                &param, src, scope_tree, symbols, refs, parent_index,
                            );
                        }
                    }
                }
            }
            break;
        }
    }
}

fn extract_class_parameter(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Walk children to find identifier and type.
    // NOTE: In tree-sitter-kotlin-ng the `type` rule is transparent — the actual
    // child node kind is `user_type`, `nullable_type`, `function_type`, etc.
    let mut name: Option<String> = None;
    let mut type_node: Option<tree_sitter::Node> = None;
    let mut is_property = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "simple_identifier" => {
                if name.is_none() {
                    name = Some(node_text(child, src));
                }
            }
            // Explicit `type` wrapper (if present in some grammar versions).
            "type" | "nullable_type" => {
                type_node = Some(child);
            }
            // In kotlin-ng the type rule is transparent — `user_type` appears directly.
            "user_type" | "function_type" => {
                type_node = Some(child);
            }
            // `val` or `var` keyword makes this a promoted property.
            "val" | "var" => {
                is_property = true;
            }
            _ => {}
        }
    }

    let name = match name {
        Some(n) => n,
        None => return,
    };

    // Emit TypeRef for the parameter type.
    if let Some(tn) = type_node {
        // Extract the simple name from the type node and emit a TypeRef directly.
        let type_name = super::calls::kotlin_type_name(&tn, src);
        if !type_name.is_empty() {
            refs.push(crate::types::ExtractedRef {
                source_symbol_index: parent_index.unwrap_or(0),
                target_name: type_name,
                kind: crate::types::EdgeKind::TypeRef,
                line: tn.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
            });
        }
    }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if is_property {
        SymbolKind::Property
    } else {
        SymbolKind::Variable
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

/// Emit TypeRef edges for upper bounds of `type_parameter` nodes inside a
/// `type_parameters` list (e.g. `<T : Comparable<T>>`).
pub(super) fn extract_type_parameter_bounds(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_parameters" {
            let mut tc = child.walk();
            for tp in child.children(&mut tc) {
                if tp.kind() == "type_parameter" {
                    // In Kotlin-ng 1.1, `type` is a supertype — the actual bound node
                    // is a concrete subtype (user_type, nullable_type, function_type, etc.).
                    // Iterate ALL children and emit TypeRef for any known type node.
                    let mut ic = tp.walk();
                    for inner in tp.children(&mut ic) {
                        match inner.kind() {
                            "type" | "user_type" | "nullable_type" | "function_type"
                            | "non_nullable_type" | "parenthesized_type" => {
                                super::calls::extract_type_ref_from_type_node(
                                    &inner,
                                    src,
                                    source_symbol_index,
                                    refs,
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn push_secondary_constructor(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("constructor").to_string();
    let qualified_name = scope_tree::qualify(&class_name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let params = find_child_by_kind(node, "function_value_parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: class_name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("constructor{params}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_imports(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_header" {
            emit_import(&child, src, current_symbol_count, refs);
        }
    }
}

pub(super) fn emit_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" => {
                let mut parts: Vec<String> = Vec::new();
                let mut ic = child.walk();
                for id in child.children(&mut ic) {
                    if id.kind() == "identifier" {
                        parts.push(node_text(id, src));
                    }
                }
                if parts.is_empty() {
                    let full = node_text(child, src);
                    let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: Some(full),
                        chain: None,
                        byte_offset: 0,
                    });
                } else {
                    let target = parts.last().cloned().unwrap_or_default();
                    let full = parts.join(".");
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: Some(full),
                        chain: None,
                        byte_offset: 0,
                    });
                }
                return;
            }
            "identifier" => {
                let full = node_text(child, src);
                let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                    chain: None,
                    byte_offset: 0,
                });
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Inheritance / interface delegation
// ---------------------------------------------------------------------------

pub(super) fn extract_delegation_specifiers(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut first_super = true;
    for child in node.children(&mut cursor) {
        if child.kind() == "delegation_specifiers" {
            let mut dc = child.walk();
            for spec in child.children(&mut dc) {
                match spec.kind() {
                    "delegation_specifier" | "annotated_delegation_specifier" => {
                        if let Some(name) = delegation_spec_name(&spec, src) {
                            let kind = if first_super {
                                first_super = false;
                                EdgeKind::Inherits
                            } else {
                                EdgeKind::Implements
                            };
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: name,
                                kind,
                                line: spec.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn delegation_spec_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "constructor_invocation" => {
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "user_type" {
                        return last_simple_identifier_in_user_type(&inner, src);
                    }
                }
            }
            "user_type" => {
                return last_simple_identifier_in_user_type(&child, src);
            }
            "simple_identifier" | "type_identifier" => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    None
}

fn last_simple_identifier_in_user_type(node: &Node, src: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "simple_user_type" => {
                if let Some(id_node) = child.child_by_field_name("name") {
                    last = Some(node_text(id_node, src));
                } else {
                    let mut ic = child.walk();
                    for inner in child.children(&mut ic) {
                        if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                            last = Some(node_text(inner, src));
                            break;
                        }
                    }
                }
            }
            "identifier" | "simple_identifier" => {
                last = Some(node_text(child, src));
            }
            _ => {}
        }
    }
    last
}

/// Extract a getter declaration as a Method symbol.
/// In Kotlin, getters appear as children of property_declaration with kind "getter".
/// Shape: getter → ("get" keyword) + optional modifiers + optional type + function_body
pub(super) fn push_getter_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Getter is a method-like accessor; name it after its enclosing property.
    // Try to find the property name from parent scope or use "get" as fallback.
    let name = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| format!("get_{}", s.name))
        .unwrap_or_else(|| "get".to_string());

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some("get()".to_string()),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

/// Extract a setter declaration as a Method symbol.
/// In Kotlin, setters appear as children of property_declaration with kind "setter".
/// Shape: setter → ("set" keyword) + optional modifiers + optional parameter + function_body
pub(super) fn push_setter_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Setter is a method-like accessor; name it after its enclosing property.
    // Try to find the property name from parent scope or use "set" as fallback.
    let name = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| format!("set_{}", s.name))
        .unwrap_or_else(|| "set".to_string());

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    // Extract parameter name if present (typically "value").
    let param_name = find_child_by_kind(node, "parameter")
        .and_then(|p| p.child_by_field_name("name"))
        .map(|n| node_text(n, src))
        .unwrap_or_else(|| "value".to_string());

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("set({})", param_name)),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}
