// =============================================================================
// csharp/symbols/specials.rs  —  special-member declarations + using directives
//
// Pushes Indexer, Operator, Conversion operator, Destructor, LocalFunction,
// and Event symbols, plus emits Imports edges for `using` directives.
// =============================================================================

use super::super::helpers::{detect_visibility, extract_doc_comment, node_text};
use super::super::types::{extract_type_refs_from_params, extract_type_refs_from_type_node};
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Indexer, operator, conversion operator, destructor, local function, event
// ---------------------------------------------------------------------------

/// `this[int index]` — extract as a Property symbol named `this[]`.
pub(in super::super) fn push_indexer_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify("this[]", parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: "this[]".to_string(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{type_str} this[]")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // TypeRef for return type and parameter types.
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_from_type_node(type_node, src, idx, refs);
    }
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, idx, refs);
    }
    Some(idx)
}

/// `public static Foo operator +(Foo a, Foo b)` — emit as Method with name like `operator+`.
pub(in super::super) fn push_operator_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // The operator token is an unnamed child after the `operator` keyword.
    // Find the operator symbol text.
    let op_text = {
        let mut after_op_kw = false;
        let mut found = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "operator" {
                after_op_kw = true;
                continue;
            }
            if after_op_kw && child.kind() != "(" {
                found = node_text(child, src);
                break;
            }
        }
        found
    };
    if op_text.is_empty() {
        return None;
    }
    let name = format!("operator{op_text}");

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
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("operator {op_text}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // TypeRef for return type and parameters.
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_from_type_node(type_node, src, idx, refs);
    }
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, idx, refs);
    }
    Some(idx)
}

/// `implicit operator int(Foo f)` / `explicit operator Foo(int i)` — emit as Method.
pub(in super::super) fn push_conversion_operator_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // The conversion kind is either `implicit` or `explicit` (first token).
    // The target type is the `type` field.
    let kind_kw = {
        let mut cursor = node.walk();
        let mut found = "conversion".to_string();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "implicit" | "explicit") {
                found = node_text(child, src);
                break;
            }
        }
        found
    };
    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let name = format!("{kind_kw} operator {type_str}");

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
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(name),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_from_type_node(type_node, src, idx, refs);
    }
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, idx, refs);
    }
    Some(idx)
}

/// `~ClassName()` — destructor, emitted as a Method.
pub(in super::super) fn push_destructor_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let class_name = node_text(name_node, src);
    let name = format!("~{class_name}");

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
        kind: SymbolKind::Method,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("~{class_name}()")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Local function inside a method body — emitted as Function.
pub(in super::super) fn push_local_function_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
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
        .child_by_field_name("returns")
        .map(|r| node_text(r, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{ret} {name}{params}").trim().to_string()),
        doc_comment: None,
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// `event EventHandler Clicked { add { ... } remove { ... } }` — event with accessors.
pub(in super::super) fn push_event_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
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

    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Event,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("event {type_str} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_from_type_node(type_node, src, idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Import / using directive
// ---------------------------------------------------------------------------

pub(in super::super) fn push_using_directive(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // For `using Alias = Full.Name;` — emit an Imports edge for the RHS namespace.
    // tree-sitter-c-sharp: using_directive has a `name` field for the alias and
    // the RHS is a `qualified_name` or `identifier` sibling after `=`.
    if node.child_by_field_name("name").is_some() {
        // Walk children looking for the RHS (qualified_name or identifier after `=`).
        let mut past_eq = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "=" {
                past_eq = true;
                continue;
            }
            if past_eq {
                match child.kind() {
                    "identifier" | "qualified_name" => {
                        let full = node_text(child, src);
                        refs.push(ExtractedRef {
                            source_symbol_index: current_symbol_count,
                            target_name: full.clone(),
                            kind: EdgeKind::Imports,
                            line: child.start_position().row as u32,
                            module: Some(full),
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                        return;
                    }
                    ";" => return,
                    _ => {}
                }
            }
        }
        return;
    }

    // Normal using directive: `using System.Linq;`
    // Extract the full namespace path and emit a single Imports edge.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(name),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
                return;
            }
            "qualified_name" => {
                let full = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: full.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
                return;
            }
            _ => {}
        }
    }
}
