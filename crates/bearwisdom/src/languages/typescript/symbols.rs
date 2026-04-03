use super::calls::build_chain;
use super::helpers::{detect_visibility, extract_jsdoc, node_text};
use super::types::{extract_type_ref_from_annotation, extract_type_refs_recursive};
use crate::parser::scope_tree;
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, SegmentKind, SymbolKind,
};
use tree_sitter::Node;

pub(super) fn push_class(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
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
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

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
        signature: Some(format!("class {name}")),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_interface(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
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
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Interface,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("interface {name}")),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_function(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
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
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| node_text(r, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("function {name}{params}{ret}").trim().to_string()),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Emit a Constructor symbol for `construct_signature` in an interface.
///
/// `interface Factory { new(name: string): Product; }` — no `name` field,
/// so we synthesise the name `new`.
pub(super) fn push_construct_signature(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify("new", parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: "new".to_string(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Emit a Method symbol for `call_signature` in an interface.
///
/// `interface Callable { (x: number): string; }` — no `name` field, synthesise `call`.
pub(super) fn push_call_signature(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify("call", parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: "call".to_string(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_method(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
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
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let kind = if name == "constructor" {
        SymbolKind::Constructor
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
        signature: None,
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_ts_field(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Property,
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

    // Extract TypeRef from field type annotation: `db: DatabaseRepository`
    if let Some(type_ann) = node.child_by_field_name("type") {
        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
    }
}

pub(super) fn push_enum(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Enum,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Enum members.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            if member.kind() == "enum_assignment"
                || member.kind() == "property_identifier"
                || member.kind() == "identifier"
            {
                let mname_node = if member.kind() == "enum_assignment" {
                    member.child_by_field_name("name")
                } else {
                    Some(member)
                };
                if let Some(mn) = mname_node {
                    let mname = node_text(mn, src);
                    symbols.push(ExtractedSymbol {
                        name: mname.clone(),
                        qualified_name: format!("{qualified_name}.{mname}"),
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: member.start_position().row as u32,
                        end_line: member.end_position().row as u32,
                        start_col: member.start_position().column as u32,
                        end_col: member.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: Some(qualified_name.clone()),
                        parent_index: Some(idx),
                    });
                }
            }
        }
    }
}

pub(super) fn push_type_alias(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("type {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });

    // Extract TypeRef from type alias value: `type UserId = string`
    if let Some(value) = node.child_by_field_name("value") {
        extract_type_ref_from_annotation(&value, src, idx, refs);
    }
}

/// Extract a TypeRef from `user as Admin` — the `as_expression` node.
///
/// Tree-sitter structure:
/// ```text
/// as_expression
///   identifier "user"      ← expression
///   "as"
///   type_identifier "Admin" ← asserted type
/// ```
/// We look for a `type_identifier`, `generic_type`, or `identifier` child that
/// appears after the `as` keyword.
pub(super) fn extract_type_ref_from_as_expression(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut after_as = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "as" {
            after_as = true;
            continue;
        }
        if !after_as {
            continue;
        }
        // First node after `as` is the asserted type — delegate to the full
        // annotation handler so all complex type forms (union, generic, etc.) work.
        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
        return;
    }
}

/// Extract a TypeRef from `expr satisfies TypeName` — the `satisfies_expression` node.
///
/// Tree-sitter structure:
/// ```text
/// satisfies_expression
///   <expression>           ← the value being checked
///   "satisfies"
///   type_identifier "Config"  ← the asserted type
/// ```
pub(super) fn extract_type_ref_from_satisfies_expression(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut after_satisfies = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "satisfies" {
            after_satisfies = true;
            continue;
        }
        if !after_satisfies {
            continue;
        }
        // First node after `satisfies` is the asserted type.
        // Delegate to the full annotation handler so all complex type forms
        // (union, generic, intersection, conditional, etc.) are covered.
        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
        return;
    }
}

/// Extract a TypeRef from `<Admin>user` — the `type_assertion` node.
///
/// Tree-sitter structure:
/// ```text
/// type_assertion
///   type_arguments
///     type_identifier "Admin"
///   identifier "user"
/// ```
pub(super) fn extract_type_ref_from_type_assertion(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let type_args = if let Some(n) = node.child_by_field_name("type_arguments") {
        n
    } else {
        // Fallback: find the first child of kind "type_arguments".
        let mut found: Option<Node> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_arguments" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return,
        }
    };

    let mut cursor = type_args.walk();
    for child in type_args.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let type_name = node_text(child, src);
                if !type_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                return;
            }
            "generic_type" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let type_name = node_text(name_node, src);
                    if !type_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: type_name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                return;
            }
            _ => {}
        }
    }
}

/// Emit a Namespace symbol for `namespace NS { ... }` / `module NS { ... }`.
///
/// Tree-sitter represents these as `internal_module` with fields:
/// - `name`: identifier (the namespace name)
/// - `body`: statement_block (the contents)
pub(super) fn push_namespace(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
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
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

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
        signature: Some(format!("namespace {name}")),
        doc_comment: extract_jsdoc(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Emit a Property symbol for an index signature: `[key: string]: Value`.
///
/// Tree-sitter structure:
/// ```text
/// index_signature
///   "[" "[" identifier ":" type_annotation "]"
///   ":" type_annotation
/// ```
/// We emit a symbol named `[index]` (or keyed by the parameter name) and
/// extract TypeRef for the value type (child after the closing `]`).
pub(super) fn push_index_signature(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Find the key parameter name (first identifier inside the brackets).
    let key_name = {
        let mut found = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                found = Some(node_text(child, src));
                break;
            }
        }
        found.unwrap_or_else(|| "index".to_string())
    };
    let name = format!("[{key_name}]");

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Property,
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

    // Extract TypeRef from the value type annotation (the type after the closing `]:`).
    // The index_signature has a `type` field for the value type in tree-sitter.
    if let Some(type_ann) = node.child_by_field_name("type") {
        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
    } else {
        // Fallback: scan for the last type_annotation child.
        let mut last_type_ann: Option<Node> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_annotation" {
                last_type_ann = Some(child);
            }
        }
        if let Some(ann) = last_type_ann {
            extract_type_ref_from_annotation(&ann, src, idx, refs);
        }
    }
}

pub(super) fn push_variable_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    // `const Foo = ...` — get declarators.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                // Capture simple identifiers and object destructuring patterns.
                if name_node.kind() == "identifier" {
                    let name = node_text(name_node, src);
                    let qualified_name = scope_tree::qualify(&name, parent_scope);
                    let idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name,
                        kind: SymbolKind::Variable,
                        visibility: detect_visibility(node, src),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: Some(format!("const {name}")),
                        doc_comment: None,
                        scope_path: scope_path.clone(),
                        parent_index,
                    });

                    // Extract TypeRef from variable type annotation: `const repo: Repository`
                    if let Some(type_ann) = child.child_by_field_name("type") {
                        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
                    } else if let Some(init) = child.child_by_field_name("value") {
                        // No explicit type — try to infer from initializer.
                        // `const user = this.repo.findOne(1)` → chain [this, repo, findOne]
                        // Emit a chain-bearing TypeRef so the index builder can
                        // resolve the chain's return type as the variable's type.
                        let init_node = if init.kind() == "await_expression" {
                            // `const user = await this.repo.findOne(1)` → unwrap await
                            init.child_by_field_name("value")
                                .or_else(|| init.named_child(0))
                                .unwrap_or(init)
                        } else {
                            init
                        };
                        if init_node.kind() == "call_expression" {
                            if let Some(func) = init_node.child_by_field_name("function") {
                                if let Some(chain) = build_chain(func, src) {
                                    // Use the last segment as the target_name.
                                    let target = chain
                                        .segments
                                        .last()
                                        .map(|s| s.name.clone())
                                        .unwrap_or_default();
                                    if !target.is_empty() {
                                        refs.push(ExtractedRef {
                                            source_symbol_index: idx,
                                            target_name: target,
                                            kind: EdgeKind::TypeRef,
                                            line: init_node.start_position().row as u32,
                                            module: None,
                                            chain: Some(chain),
                                        });
                                    }
                                }
                            }
                        } else if init_node.kind() == "new_expression" {
                            // `const map = new Map()` → type is the constructor name
                            if let Some(constructor) = init_node.child_by_field_name("constructor") {
                                let type_name = match constructor.kind() {
                                    "identifier" | "type_identifier" => {
                                        node_text(constructor, src)
                                    }
                                    _ => String::new(),
                                };
                                if !type_name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: idx,
                                        target_name: type_name,
                                        kind: EdgeKind::TypeRef,
                                        line: init_node.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                    });
                                }
                            }
                        } else if init_node.kind() == "member_expression" {
                            // `const x = obj.field` → chain for field type inference
                            if let Some(chain) = build_chain(init_node, src) {
                                let target = chain
                                    .segments
                                    .last()
                                    .map(|s| s.name.clone())
                                    .unwrap_or_default();
                                if !target.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: idx,
                                        target_name: target,
                                        kind: EdgeKind::TypeRef,
                                        line: init_node.start_position().row as u32,
                                        module: None,
                                        chain: Some(chain),
                                    });
                                }
                            }
                        } else if init_node.kind() == "as_expression" {
                            // `const admin = user as Admin` → type is Admin
                            // The type node is the last named child after the `as` keyword.
                            extract_type_ref_from_as_expression(&init_node, src, idx, refs);
                        } else if init_node.kind() == "type_assertion" {
                            // `const admin = <Admin>user` → type is Admin
                            // type_assertion has type_arguments as first child.
                            extract_type_ref_from_type_assertion(&init_node, src, idx, refs);
                        } else if init_node.kind() == "satisfies_expression" {
                            // `const obj = { a: 1 } satisfies Config` → type is Config
                            // satisfies_expression: <expr> satisfies <type>
                            // The type is the last named child after the `satisfies` keyword.
                            extract_type_ref_from_satisfies_expression(
                                &init_node, src, idx, refs,
                            );
                        }
                    }
                } else if name_node.kind() == "object_pattern" {
                    // const { name, email } = user
                    // Extract each destructured property as a Variable symbol
                    // with a TypeRef chain to the source expression.
                    let source_chain = child
                        .child_by_field_name("value")
                        .and_then(|init| build_chain(init, src));

                    let mut ppcursor = name_node.walk();
                    for prop in name_node.children(&mut ppcursor) {
                        let prop_name = if prop.kind()
                            == "shorthand_property_identifier_pattern"
                            || prop.kind() == "shorthand_property_identifier"
                        {
                            node_text(prop, src)
                        } else if prop.kind() == "pair_pattern" {
                            prop.child_by_field_name("key")
                                .map(|k| node_text(k, src))
                                .unwrap_or_default()
                        } else {
                            continue;
                        };
                        if prop_name.is_empty() {
                            continue;
                        }

                        let qualified_name = scope_tree::qualify(&prop_name, parent_scope);
                        let prop_idx = symbols.len();
                        symbols.push(ExtractedSymbol {
                            name: prop_name.clone(),
                            qualified_name,
                            kind: SymbolKind::Variable,
                            visibility: detect_visibility(node, src),
                            start_line: prop.start_position().row as u32,
                            end_line: prop.end_position().row as u32,
                            start_col: prop.start_position().column as u32,
                            end_col: prop.end_position().column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_path.clone(),
                            parent_index,
                        });

                        // Emit chain to source with property name appended so the
                        // index builder can resolve the type of this property.
                        if let Some(ref base_chain) = source_chain {
                            let mut prop_chain = base_chain.clone();
                            prop_chain.segments.push(ChainSegment {
                                name: prop_name.clone(),
                                node_kind: "property".to_string(),
                                kind: SegmentKind::Property,
                                declared_type: None,
                                type_args: vec![],
                                optional_chaining: false,
                            });
                            refs.push(ExtractedRef {
                                source_symbol_index: prop_idx,
                                target_name: prop_name,
                                kind: EdgeKind::TypeRef,
                                line: prop.start_position().row as u32,
                                module: None,
                                chain: Some(prop_chain),
                            });
                        }
                    }
                }
            }
        }
    }
}

