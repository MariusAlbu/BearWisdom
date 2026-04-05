// =============================================================================
// python/calls.rs  —  Call extraction and import helpers for Python
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let func_name = node_text(&func_node, source);

                // `isinstance(user, Admin)` — emit TypeRef to the second argument.
                // Also emit a Calls edge so the coverage engine's `call` node budget
                // is satisfied (isinstance IS a call, just with extra semantics).
                if func_name == "isinstance" {
                    extract_isinstance_type_ref(&child, source, source_symbol_index, refs);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: "isinstance".to_string(),
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                    continue;
                }

                let chain = build_chain(&func_node, source);
                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .or_else(|| {
                        let t = node_text(&func_node, source);
                        Some(t.rsplit('.').next().unwrap_or(&t).to_string())
                    });

                crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
                if let Some(target_name) = target_name {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }
        }
        extract_calls_from_body(&child, source, source_symbol_index, refs);
    }
}

/// Emit TypeRef edges for `isinstance(obj, SomeClass)` or
/// `isinstance(obj, (ClassA, ClassB))`.
///
/// Python `call` node structure:
/// ```text
/// call
///   function: identifier "isinstance"
///   arguments: argument_list
///     identifier "obj"
///     "," (anonymous)
///     identifier "Admin"       ← single type
///     -- or --
///     tuple "(" identifier "Admin" "," identifier "User" ")"
/// ```
fn extract_isinstance_type_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let args = match node.child_by_field_name("arguments") {
        Some(a) => a,
        None => return,
    };

    // Collect all named argument children (skip commas / parens).
    let named_args: Vec<_> = {
        let mut cursor = args.walk();
        args.children(&mut cursor)
            .filter(|c| c.is_named() && c.kind() != "comment")
            .collect()
    };

    // Second argument (index 1) is the type or tuple of types.
    let type_arg = match named_args.get(1) {
        Some(a) => *a,
        None => return,
    };

    emit_isinstance_type_node(&type_arg, source, source_symbol_index, refs);
}

/// Emit TypeRef(s) for a type argument in `isinstance` — handles both a single
/// type identifier and a tuple of types `(Admin, User)`.
fn emit_isinstance_type_node(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() {
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
        // `isinstance(x, (Admin, User))` — tuple of types.
        "tuple" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(&child, source);
                    if !name.is_empty() {
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
            }
        }
        // `isinstance(x, pkg.MyClass)` — attribute access.
        "attribute" => {
            let name = node_text(node, source);
            if !name.is_empty() {
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
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

pub(super) fn build_chain(node: &Node, src: &str) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, src);
            let kind = if name == "self" || name == "cls" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&attribute, src),
                node_kind: "attribute".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call" => {
            let func = node.child_by_field_name("function")?;
            build_chain_inner(&func, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_import_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let full = node_text(&child, source);
                let parts: Vec<&str> = full.split('.').collect();
                let target = parts.last().unwrap_or(&full.as_str()).to_string();
                let module = if parts.len() > 1 {
                    Some(parts[..parts.len() - 1].join("."))
                } else {
                    None
                };
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module,
                    chain: None,
                });
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let full = node_text(&name_node, source);
                    let parts: Vec<&str> = full.split('.').collect();
                    let target = parts.last().unwrap_or(&full.as_str()).to_string();
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("."))
                    } else {
                        None
                    };
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module,
                        chain: None,
                    });
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_import_from_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let module = node.child_by_field_name("module_name").map(|m| {
        node_text(&m, source).trim_start_matches('.').to_string()
    });

    let module_name_node = node.child_by_field_name("module_name");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "from" | "import" | "," | "import_prefix" => continue,
            _ => {}
        }
        if let Some(ref mn) = module_name_node {
            if child.id() == mn.id() {
                continue;
            }
        }

        match child.kind() {
            "dotted_name" | "identifier" => {
                let name = node_text(&child, source);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: module.clone(),
                    chain: None,
                });
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, source);
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: module.clone(),
                        chain: None,
                    });
                }
            }
            "wildcard_import" => {
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: "*".to_string(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: module.clone(),
                    chain: None,
                });
            }
            _ => {}
        }
    }
}
