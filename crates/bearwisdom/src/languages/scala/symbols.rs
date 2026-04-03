// =============================================================================
// scala/symbols.rs  —  Symbol pushers and body recursion for Scala
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, node_text, type_name_from_node,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Body recursion
// ---------------------------------------------------------------------------

pub(super) fn recurse_body(
    type_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    if let Some(body) = type_node.child_by_field_name("body") {
        super::extract::extract_node(body, src, scope_tree, symbols, refs, parent_index);
    } else {
        // Scan for template_body or class_body children.
        let mut cursor = type_node.walk();
        for child in type_node.children(&mut cursor) {
            match child.kind() {
                "template_body" | "class_body" => {
                    super::extract::extract_node(child, src, scope_tree, symbols, refs, parent_index);
                }
                _ => {}
            }
        }
    }
}

pub(super) fn extract_enum_body(
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
        if child.kind() == "enum_body" {
            let mut cursor = child.walk();
            for item in child.children(&mut cursor) {
                match item.kind() {
                    "enum_case_definitions" => {
                        // enum_case_definitions → enum_case_definition*
                        let mut ic = item.walk();
                        for case_def in item.children(&mut ic) {
                            if case_def.kind() == "enum_case_definition" {
                                if let Some(name_node) = case_def.child_by_field_name("name") {
                                    let name = node_text(name_node, src);
                                    let qualified_name = if enum_qname.is_empty() {
                                        name.clone()
                                    } else {
                                        format!("{enum_qname}.{name}")
                                    };
                                    let scope = enclosing_scope(
                                        scope_tree,
                                        case_def.start_byte(),
                                        case_def.end_byte(),
                                    );
                                    symbols.push(ExtractedSymbol {
                                        name,
                                        qualified_name,
                                        kind: SymbolKind::EnumMember,
                                        visibility: None,
                                        start_line: case_def.start_position().row as u32,
                                        end_line: case_def.end_position().row as u32,
                                        start_col: case_def.start_position().column as u32,
                                        end_col: case_def.end_position().column as u32,
                                        signature: None,
                                        doc_comment: None,
                                        scope_path: scope_tree::scope_path(scope),
                                        parent_index,
                                    });
                                }
                            }
                        }
                    }
                    // Other items in enum body (defs, vals, etc.).
                    _ => {
                        super::extract::extract_node(item, src, scope_tree, symbols, refs, parent_index);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

pub(super) fn push_type_def(
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
        SymbolKind::Namespace => "object",
        SymbolKind::Interface => "trait",
        SymbolKind::Enum      => "enum",
        _                     => "class",
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
        signature: Some(format!("{kw} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_function_def(
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
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(": {}", node_text(r, src)))
        .unwrap_or_default();
    let signature = Some(format!("def {name}{params}{ret}"));

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

pub(super) fn push_val_var(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // val_definition: pattern field or first identifier child.
    let name_opt = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            // Pattern may be typed_pattern → identifier.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => return Some(node_text(child, src)),
                    "typed_pattern" => {
                        let mut pc = child.walk();
                        for inner in child.children(&mut pc) {
                            if inner.kind() == "identifier" {
                                return Some(node_text(inner, src));
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        });

    let name = match name_opt {
        Some(n) => n,
        None => return,
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let text = node_text(*node, src);
    let kw = if text.trim_start().starts_with("val") { "val" } else { "var" };
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

/// Emit a TypeAlias symbol for a Scala `type` definition.
/// `type_definition` has `name` and optionally `type` fields.
pub(super) fn push_type_definition(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

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
        signature: Some(format!("type {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit TypeRef for the aliased type (field `type`).
    if let Some(type_node) = node.child_by_field_name("type") {
        let alias_name = type_name_from_node(&type_node, src);
        if !alias_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: alias_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }

    Some(idx)
}

/// Emit a Class symbol for a `given_definition` (Scala 3 implicit instance).
/// `given_definition` has an optional `name` field and a `return_type` field.
pub(super) fn push_given_definition(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // Name is optional — use return_type name as fallback.
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            node.child_by_field_name("return_type")
                .map(|t| type_name_from_node(&t, src))
                .filter(|n| !n.is_empty())
        })?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("given {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit TypeRef for the given's return_type.
    if let Some(rt) = node.child_by_field_name("return_type") {
        let type_name = type_name_from_node(&rt, src);
        if !type_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: idx,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: rt.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }

    Some(idx)
}

/// Emit a Namespace symbol for `extension (T) { ... }` (Scala 3).
/// `extension_definition` has `parameters` (the extended type) but no `name`.
pub(super) fn push_extension_definition(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // The extended type lives inside the `parameters` field.
    let name = node
        .child_by_field_name("parameters")
        .and_then(|p| {
            let mut cursor = p.walk();
            for child in p.named_children(&mut cursor) {
                // parameters → parameters → parameter → type
                if child.kind() == "parameters" {
                    let mut ic = child.walk();
                    for param in child.named_children(&mut ic) {
                        // Each param may have a `type` child.
                        if let Some(type_node) = param.child_by_field_name("type") {
                            let n = type_name_from_node(&type_node, src);
                            if !n.is_empty() {
                                return Some(n);
                            }
                        }
                    }
                }
                let n = type_name_from_node(&child, src);
                if !n.is_empty() {
                    return Some(n);
                }
            }
            None
        })
        .unwrap_or_else(|| "extension".to_string());

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

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_declaration children: `import`, stable_id, import_selectors?
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_expression" => {
                emit_import_expression(&child, src, current_symbol_count, refs);
            }
            "stable_id" | "identifier" => {
                let full = node_text(child, src);
                let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                    chain: None,
                });
            }
            _ => {}
        }
    }
}

fn emit_import_expression(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // import_expression → stable_id, import_selectors?
    let mut cursor = node.walk();
    let mut base: Option<String> = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "stable_id" | "identifier" => {
                base = Some(node_text(child, src));
            }
            "import_selectors" | "named_imports" => {
                // { Foo, Bar, ... }
                let base_path = base.as_deref().unwrap_or("");
                let mut sc = child.walk();
                for sel in child.children(&mut sc) {
                    if sel.kind() == "import_selector" || sel.kind() == "identifier" {
                        let name_node = sel.child_by_field_name("name").unwrap_or(sel);
                        let name = node_text(name_node, src);
                        let module = if base_path.is_empty() {
                            name.clone()
                        } else {
                            format!("{base_path}.{name}")
                        };
                        refs.push(ExtractedRef {
                            source_symbol_index: current_symbol_count,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line: sel.start_position().row as u32,
                            module: Some(module),
                            chain: None,
                        });
                    }
                }
                return;
            }
            _ => {}
        }
    }
    // No selectors — emit for the stable_id itself.
    if let Some(full) = base {
        let target = full.rsplit('.').next().unwrap_or(&full).to_string();
        refs.push(ExtractedRef {
            source_symbol_index: current_symbol_count,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(full),
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Extends / with extraction
// ---------------------------------------------------------------------------

/// Extract `extends T1 with T2 with T3` from a type definition.
///
/// First parent → Inherits, subsequent `with` mixins → Implements.
pub(super) fn extract_extends_with(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut first_extends = true;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "extends_clause" => {
                let mut ec = child.walk();
                for type_node in child.children(&mut ec) {
                    let name = type_name_from_node(&type_node, src);
                    if !name.is_empty() {
                        let kind = if first_extends {
                            first_extends = false;
                            EdgeKind::Inherits
                        } else {
                            EdgeKind::Implements
                        };
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            // `with` mixins (Scala 2 style: `extends Base with Mixin`)
            "with_clause" => {
                let mut wc = child.walk();
                for type_node in child.children(&mut wc) {
                    let name = type_name_from_node(&type_node, src);
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
            _ => {}
        }
    }
}
