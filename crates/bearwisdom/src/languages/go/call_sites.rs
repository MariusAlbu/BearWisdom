// =============================================================================
// go/call_sites.rs  —  Per-construct ref emitters for Go
//
// Emits the actual `ExtractedRef` records for individual expression kinds:
//   * `extract_call_ref`          — `call_expression`
//   * `extract_make_chan_type_ref`— `make(chan T)` TypeRef edge
//   * `extract_composite_literal_ref` — `composite_literal` Instantiates edge
//   * `extract_type_assertion_ref`— `x.(*Admin)` TypeRef edge
//   * `extract_type_switch_refs`  — `switch x.(type) { case *T: ... }` TypeRef
// =============================================================================

use super::calls::extract_call_args;
use super::chain::build_chain;
use super::helpers::node_text;
use super::qualified_types::go_type_ref_target;
use super::refs::{extract_func_literal_type_refs, extract_refs_from_body};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Emit a `Calls` ref for a `call_expression`.
///
/// `call_expression` children (positional):
///   function (identifier | selector_expression | ...), argument_list
///
/// For `bar.Baz()` the function part is a `selector_expression` with children:
///   operand, `.`, `field_identifier`
///
/// Special case: `make(chan User, 10)` — emit a TypeRef for the channel element
/// type in addition to the normal Calls edge.
pub(super) fn extract_call_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The function part is the first named child (use index to avoid cursor borrow).
    let func_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Anonymous-function IIFE: `func(...) { ... }(args)`. The function is
    // defined and invoked in place — no resolvable name. Walk the callee's
    // body so nested call/type refs still surface; the call's own arguments
    // are covered by the `call_expression` dispatch arm after this returns.
    // Skip emitting a `Calls` edge with the literal source as the target.
    if matches!(
        func_node.kind(),
        "func_literal" | "parenthesized_expression"
    ) {
        if func_node.kind() == "func_literal" {
            extract_func_literal_type_refs(&func_node, source, source_symbol_index, refs);
            if let Some(body) = func_node.child_by_field_name("body") {
                extract_refs_from_body(&body, source, source_symbol_index, refs);
            }
        } else {
            extract_refs_from_body(&func_node, source, source_symbol_index, refs);
        }
        return;
    }

    // Generic instantiation: `Foo[T](args)` or `pkg.Foo[T](args)` are wrapped
    // in `index_expression`. Peel it off so the callee resolves to the
    // unparameterized name (`Foo` / `pkg.Foo`) instead of `Foo[T]`.
    let func_node = if func_node.kind() == "index_expression" {
        func_node
            .child_by_field_name("operand")
            .unwrap_or(func_node)
    } else {
        func_node
    };

    let func_name = node_text(&func_node, source);

    // `make(chan T, ...)` — extract the channel element type as a TypeRef.
    if func_name == "make" {
        extract_make_chan_type_ref(node, source, source_symbol_index, refs);
    }

    // Build a structured chain for selector expressions; fall back to the
    // existing single-name extraction for bare identifiers.
    let chain = build_chain(func_node, source);

    let target_name = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| match func_node.kind() {
            "selector_expression" => (0..func_node.named_child_count())
                .filter_map(|i| func_node.named_child(i))
                .find(|c| c.kind() == "field_identifier")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(&func_node, source)),
            _ => func_name.clone(),
        });

    if target_name.is_empty() {
        return;
    }

    // When the callee is a bare identifier that starts with an uppercase letter,
    // Go convention says it is exported — this may be a user-defined type
    // conversion (`MyString(b)` is syntactically a call_expression in tree-sitter-go,
    // not a type_conversion_expression).  Emit a TypeRef so the resolution engine
    // can treat it as a potential type usage.
    if func_node.kind() == "identifier"
        && target_name
            .chars()
            .next()
            .map_or(false, |c| c.is_uppercase())
        && !super::helpers::is_go_builtin_type(&target_name)
    {
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: target_name.clone(),
            kind: EdgeKind::TypeRef,
            line: func_node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: func_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }

    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
    let call_args = extract_call_args(&node, source);
    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name,
        kind: EdgeKind::Calls,
        line: func_node.start_position().row as u32,
        col: 0,
        module: None,
        chain,
        byte_offset: func_node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args,
    });
}

/// For `make(chan User, 10)` emit a TypeRef to `User` (the channel element type).
///
/// Tree-sitter-go shape:
/// ```text
/// call_expression
///   identifier "make"
///   argument_list
///     channel_type
///       type_identifier "User"
///     int_literal "10"
/// ```
fn extract_make_chan_type_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let args = match (0..node.named_child_count())
        .filter_map(|i| node.named_child(i))
        .find(|c| c.kind() == "argument_list")
    {
        Some(a) => a,
        None => return,
    };

    // First argument to make() — look for a channel_type node.
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "channel_type" {
            // channel_type children: `chan` (anon), element_type
            let mut inner = child.walk();
            for elem in child.children(&mut inner) {
                if !elem.is_named() {
                    continue; // skip `chan` keyword
                }
                if let Some((elem_name, module)) = go_type_ref_target(&elem, source) {
                    if !elem_name.is_empty() {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name: elem_name,
                            kind: EdgeKind::TypeRef,
                            line: elem.start_position().row as u32,
                            col: 0,
                            module,
                            chain: None,
                            byte_offset: elem.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
                break;
            }
            break;
        }
    }
}

/// Emit an `Instantiates` ref for a `composite_literal`.
///
/// `composite_literal` children: type (identifier or qualified_type), literal_value
pub(super) fn extract_composite_literal_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The type is the first named child (use index to avoid cursor borrow).
    let type_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Skip if the first named child is the literal_value `{...}` (happens for
    // anonymous composite literals like `{1, 2}`).
    if type_node.kind() == "literal_value" {
        return;
    }

    let (type_name, module) = match type_node.kind() {
        "type_identifier" | "qualified_type" | "generic_type" => {
            match go_type_ref_target(&type_node, source) {
                Some(parts) => parts,
                None => return,
            }
        }
        // Anonymous types (`struct{}{...}`, `[]int{1,2}`, `map[K]V{}`,
        // `[N]T{}`, `chan T(...)`, `func(){}` etc.) have no named symbol
        // to point at — skip rather than emit the literal source as the
        // target name.
        "struct_type" | "slice_type" | "map_type" | "array_type" | "channel_type"
        | "function_type" | "pointer_type" | "interface_type" => return,
        _ => (node_text(&type_node, source), None),
    };

    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::Instantiates,
        line: type_node.start_position().row as u32,
        col: 0,
        module,
        chain: None,
        byte_offset: type_node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

// ---------------------------------------------------------------------------
// Type narrowing — type assertions and type switches
// ---------------------------------------------------------------------------

/// Emit a TypeRef for `x.(*Admin)` — a `type_assertion_expression`.
///
/// Tree-sitter-go structure:
/// ```text
/// type_assertion_expression
///   identifier "x"          ← operand
///   pointer_type / type_identifier / qualified_type   ← asserted type
/// ```
/// The asserted type is the last named child.
pub(super) fn extract_type_assertion_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let named_count = node.named_child_count();
    if named_count < 2 {
        return;
    }
    let type_node = match node.named_child(named_count - 1) {
        Some(n) => n,
        None => return,
    };

    let (type_name, module) = match go_type_ref_target(&type_node, source) {
        Some(parts) if !parts.0.is_empty() => parts,
        _ => return,
    };

    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::TypeRef,
        line: type_node.start_position().row as u32,
        col: 0,
        module,
        chain: None,
        byte_offset: type_node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// Emit TypeRefs for each case type in a `type_switch_statement`.
///
/// ```go
/// switch v := x.(type) {
///     case *Admin:   ...
///     case *User:    ...
/// }
/// ```
/// Tree-sitter-go: `type_switch_statement` → `type_case` children,
/// each with a `type` field (or positional type children).
pub(super) fn extract_type_switch_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_case" {
            // Each case clause can list multiple types: `case *Foo, *Bar:`
            // Walk all children for type nodes.
            let mut inner = child.walk();
            for type_child in child.children(&mut inner) {
                match type_child.kind() {
                    "type_identifier" | "pointer_type" | "qualified_type" => {
                        if let Some((name, module)) = go_type_ref_target(&type_child, source) {
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    is_include: false,
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: type_child.start_position().row as u32,
                                    col: 0,
                                    module,
                                    chain: None,
                                    byte_offset: type_child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
