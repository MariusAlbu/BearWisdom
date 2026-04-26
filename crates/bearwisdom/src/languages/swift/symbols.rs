// =============================================================================
// swift/symbols.rs  —  Symbol pushers and import/inheritance helpers for Swift
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, find_child_by_kind,
    inherited_type_name, node_text, swift_type_decl_kind,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Body recursors
// ---------------------------------------------------------------------------

pub(super) fn recurse_into_body(
    type_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let body = type_node
        .child_by_field_name("body")
        .or_else(|| find_child_by_kind(type_node, "class_body"))
        .or_else(|| find_child_by_kind(type_node, "struct_body"))
        .or_else(|| find_child_by_kind(type_node, "protocol_body"))
        .or_else(|| find_child_by_kind(type_node, "extension_body"))
        .or_else(|| find_child_by_kind(type_node, "{"));
    if let Some(b) = body {
        super::extract::extract_node(b, src, scope_tree, symbols, refs, parent_index);
    } else {
        let mut cursor = type_node.walk();
        for child in type_node.children(&mut cursor) {
            match child.kind() {
                "class_body" | "struct_body" | "protocol_body" | "extension_body"
                | "enum_body" => {
                    super::extract::extract_node(child, src, scope_tree, symbols, refs, parent_index);
                }
                _ => {}
            }
        }
    }
}

pub(super) fn recurse_enum_body(
    enum_node: &Node,
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

    let mut outer = enum_node.walk();
    for child in enum_node.children(&mut outer) {
        if child.kind() != "enum_body" && child.kind() != "enum_class_body" {
            continue;
        }
        let mut cursor = child.walk();
        for item in child.children(&mut cursor) {
            match item.kind() {
                "enum_case_declaration" => {
                    let mut ic = item.walk();
                    for case_item in item.children(&mut ic) {
                        if case_item.kind() == "enum_case_name"
                            || case_item.kind() == "enum_entry"
                        {
                            let name_node = case_item
                                .child_by_field_name("name")
                                .or_else(|| find_child_by_kind(&case_item, "simple_identifier"));
                            if let Some(nn) = name_node {
                                let name = node_text(nn, src);
                                push_enum_member(name, &enum_qname, &case_item, scope_tree, symbols, parent_index, src);
                            }
                        }
                    }
                }
                "enum_entry" => {
                    let mut ec = item.walk();
                    for id_node in item.children(&mut ec) {
                        if id_node.kind() == "simple_identifier" {
                            let name = node_text(id_node, src);
                            push_enum_member(name, &enum_qname, &id_node, scope_tree, symbols, parent_index, src);
                        }
                    }
                }
                // Properties (computed or stored) inside enums — push directly.
                "property_declaration" | "stored_property" | "variable_declaration"
                | "willSet_didSet_block" | "computed_property" => {
                    let pre_len = symbols.len();
                    push_property(&item, src, scope_tree, symbols, parent_index);
                    let sym_idx = if symbols.len() > pre_len {
                        pre_len
                    } else {
                        parent_index.unwrap_or(0)
                    };
                    if symbols.len() > pre_len {
                        super::decorators::extract_decorators(&item, src, pre_len, refs);
                    }
                    super::calls::extract_all_type_identifiers_from_node(&item, src, sym_idx, refs);
                    let body = item.child_by_field_name("value")
                        .or_else(|| find_child_by_kind(&item, "computed_property"))
                        .or_else(|| find_child_by_kind(&item, "code_block"));
                    if let Some(b) = body {
                        super::calls::extract_calls_from_body(&b, src, sym_idx, refs);
                    }
                }
                // Functions and other declaration kinds — route through the standard walker.
                // `extract_node` receives a PARENT node and iterates its children.  Since `item`
                // is already the declaration node itself, we need a small wrapper: pass a synthetic
                // parent whose only child is `item` — but tree-sitter nodes don't support that.
                // Instead, call the individual dispatch functions directly for known kinds.
                "function_declaration" => {
                    let idx = push_function_decl(&item, src, scope_tree, symbols, parent_index);
                    if let Some(sym_idx) = idx {
                        super::decorators::extract_decorators(&item, src, sym_idx, refs);
                        super::extract::extract_function_type_refs(&item, src, sym_idx, refs);
                        let body = item.child_by_field_name("body")
                            .or_else(|| find_child_by_kind(&item, "code_block"));
                        if let Some(b) = body {
                            super::calls::extract_calls_from_body(&b, src, sym_idx, refs);
                        }
                    }
                }
                "initializer_declaration" | "init_declaration" => {
                    let idx = push_init(&item, src, scope_tree, symbols, parent_index);
                    if let Some(sym_idx) = idx {
                        let body = item.child_by_field_name("body")
                            .or_else(|| find_child_by_kind(&item, "code_block"))
                            .or_else(|| find_child_by_kind(&item, "function_body"));
                        if let Some(b) = body {
                            super::calls::extract_calls_from_body(&b, src, sym_idx, refs);
                        }
                    }
                }
                "typealias_declaration" => {
                    push_typealias(&item, src, scope_tree, symbols, refs, parent_index);
                }
                "subscript_declaration" => {
                    let idx = push_subscript(&item, src, scope_tree, symbols, parent_index);
                    if let Some(sym_idx) = idx {
                        let body = find_child_by_kind(&item, "computed_property")
                            .or_else(|| find_child_by_kind(&item, "code_block"));
                        if let Some(b) = body {
                            super::calls::extract_calls_from_body(&b, src, sym_idx, refs);
                        }
                    }
                }
                "class_declaration" => {
                    // Nested type declarations inside enums.
                    handle_class_declaration(&item, src, scope_tree, symbols, refs, parent_index);
                }
                _ => {
                    // For any other item, walk its children through extract_node so that
                    // nested declarations inside wrappers are still found.
                    super::extract::extract_node(item, src, scope_tree, symbols, refs, parent_index);
                }
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
        SymbolKind::Struct    => "struct",
        SymbolKind::Enum      => "enum",
        SymbolKind::Interface => "protocol",
        _                     => "class",
    };

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
        signature: Some(format!("{kw} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_extension(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name = node
        .child_by_field_name("extended_type")
        .or_else(|| find_child_by_kind(node, "user_type"))
        .or_else(|| find_child_by_kind(node, "type_identifier"))
        .map(|n| node_text(n, src))?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("extension {name}")),
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

    let kind = if scope.is_some() { SymbolKind::Method } else { SymbolKind::Function };

    let params = node
        .child_by_field_name("params")
        .or_else(|| find_child_by_kind(node, "parameter_clause"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .or_else(|| find_child_by_kind(node, "function_return_type"))
        .map(|r| format!(" -> {}", node_text(r, src)))
        .unwrap_or_default();
    let signature = Some(format!("func {name}{params}{ret}"));

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

pub(super) fn push_init(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("init").to_string();
    let qualified_name = scope_tree::qualify(&class_name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let params = find_child_by_kind(node, "parameter_clause")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: class_name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("init{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_deinit(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("deinit").to_string();
    let name = format!("~{class_name}");
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some("deinit".to_string()),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

pub(super) fn push_property(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name_opt = node
        .child_by_field_name("name")
        .and_then(|n| extract_name_from_pattern(n, src))
        .or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    return Some(node_text(child, src));
                }
                if child.kind() == "pattern" {
                    if let Some(name) = extract_name_from_pattern(child, src) {
                        return Some(name);
                    }
                }
            }
            None
        });

    let name = match name_opt {
        Some(n) => n,
        None    => return,
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let text = node_text(*node, src);
    let kw = if text.trim_start().starts_with("let") { "let" } else { "var" };
    let ty = node
        .child_by_field_name("type")
        .or_else(|| find_child_by_kind(node, "type_annotation"))
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

/// Emit a TypeAlias symbol for `typealias Name = Type`.
/// Field `name` holds the alias name; field `value` holds the aliased type.
pub(super) fn push_typealias(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // `name` field holds the alias identifier (first named child with kind simple_identifier
    // or type_identifier, not the type node).
    let name = find_alias_name(node, src)?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("typealias {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit TypeRef for the aliased type — the type node appears after `=`.
    // Walk children: after `=` take the first named node that looks like a type.
    let mut after_eq = false;
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        if child.kind() == "=" {
            after_eq = true;
            continue;
        }
        if after_eq && child.is_named() {
            super::calls::extract_type_ref_from_swift_type(&child, src, idx, refs);
            break;
        }
    }

    Some(idx)
}

/// Find the name identifier in a `typealias_declaration`.
/// The declared name is the first `type_identifier`, `simple_identifier`, or
/// `identifier` child that appears before the `=` token.
fn find_alias_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "simple_identifier" | "identifier" => {
                return Some(node_text(child, src));
            }
            // Stop at `=` token.
            "=" => break,
            _ => {}
        }
    }
    None
}

/// Emit a Method symbol for a `subscript_declaration`.
pub(super) fn push_subscript(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify("subscript", scope);
    let scope_path = scope_tree::scope_path(scope);

    let params = find_child_by_kind(node, "parameter_clause")
        .or_else(|| find_child_by_kind(node, "function_value_parameters"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(" -> {}", node_text(r, src)))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: "subscript".to_string(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("subscript{params}{ret}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_path_component" => {
                parts.push(node_text(child, src));
            }
            "identifier" => {
                parts.push(node_text(child, src));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return;
    }
    let full = parts.join(".");
    let target = parts.last().cloned().unwrap_or_else(|| full.clone());
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(full),
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

pub(super) fn extract_type_inheritance(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
    all_implements: bool,
) {
    let mut cursor = node.walk();
    let mut first = true;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_inheritance_clause" => {
                let mut ic = child.walk();
                for inherited in child.children(&mut ic) {
                    match inherited.kind() {
                        "inheritance_specifier" | "inherited_type" => {
                            if let Some(name) = inherited_type_name(&inherited, src) {
                                let kind = if all_implements || !first {
                                    EdgeKind::Implements
                                } else {
                                    EdgeKind::Inherits
                                };
                                first = false;
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: name,
                                    kind,
                                    line: inherited.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                        _ => {}
                    }
                }
            }
            "inheritance_specifier" | "inherited_type" => {
                if let Some(name) = inherited_type_name(&child, src) {
                    let kind = if all_implements || !first {
                        EdgeKind::Implements
                    } else {
                        EdgeKind::Inherits
                    };
                    first = false;
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
            _ => {}
        }
    }
}

fn push_enum_member(
    name: String,
    enum_qname: &str,
    node: &Node,
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    src: &[u8],
) {
    let qualified_name = if enum_qname.is_empty() {
        name.clone()
    } else {
        format!("{enum_qname}.{name}")
    };
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::EnumMember,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_tree::scope_path(scope),
        parent_index,
    });
}

/// Emit a TypeAlias symbol for `associatedtype Element` in a protocol.
pub(super) fn push_associatedtype(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // `associatedtype_declaration` has a `name` field or a simple_identifier/type_identifier child.
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "simple_identifier" {
                    return Some(node_text(child, src));
                }
            }
            None
        });
    let name = match name {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("associatedtype {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

/// Dispatch the `class_declaration` node to the correct handler.
pub(super) fn handle_class_declaration(
    child: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let swift_kind = swift_type_decl_kind(child, src);
    let is_enum = swift_kind == SymbolKind::Enum;
    let all_implements = swift_kind != SymbolKind::Class;
    let idx = push_type_decl(child, src, scope_tree, swift_kind, symbols, parent_index);
    if let Some(sym_idx) = idx {
        extract_type_inheritance(child, src, sym_idx, refs, all_implements);
    }
    if is_enum {
        recurse_enum_body(child, src, scope_tree, symbols, refs, idx);
    } else {
        recurse_into_body(child, src, scope_tree, symbols, refs, idx);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the bare identifier name from a Swift `pattern` node (or plain identifier).
///
/// The `pattern` node in tree-sitter-swift 0.7.1 has a `bound_identifier` field
/// that points to the `simple_identifier` holding the variable name.  For example,
/// `var name: String { get }` has name → pattern(bound_identifier: simple_identifier("name")).
pub(super) fn extract_name_from_pattern(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "simple_identifier" | "identifier" => Some(node_text(node, src)),
        "pattern" => {
            // Preferred: `bound_identifier` field.
            if let Some(bi) = node.child_by_field_name("bound_identifier") {
                let t = node_text(bi, src);
                if !t.is_empty() {
                    return Some(t);
                }
            }
            // Fallback: first simple_identifier named child.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "simple_identifier" || child.kind() == "identifier" {
                    return Some(node_text(child, src));
                }
            }
            None
        }
        _ => None,
    }
}
