// =============================================================================
// dart/calls.rs  —  Call extraction and member chain builder for Dart
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Emit a TypeRef for a Dart `type_identifier` node.
pub(super) fn emit_dart_type_ref(
    type_node: Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match type_node.kind() {
        "type_identifier" | "identifier" => node_text(type_node, src),
        _ => {
            // Walk for type_identifier inside type_cast, catch_clause, etc.
            let mut found = String::new();
            let mut cursor = type_node.walk();
            for child in type_node.named_children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "identifier" {
                    found = node_text(child, src);
                    break;
                }
            }
            found
        }
    };
    if !name.is_empty() {
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

pub(super) fn extract_dart_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Type references in type annotations, variable declarations, etc.
            // These appear throughout function bodies and class members.
            "type_identifier" => {
                emit_dart_type_ref(child, src, source_symbol_index, refs);
                // Do not recurse — type_identifier is a leaf.
            }

            // Generic type arguments: `List<MyType>`, `Map<String, MyModel>`.
            // type_arguments → type_argument_list → type_not_void (type_identifier, ...)
            "type_arguments" => {
                extract_type_arguments_refs(&child, src, source_symbol_index, refs);
            }

            // `x is MyType` — emit TypeRef for the test type.
            "type_test_expression" | "is_expression" => {
                extract_type_test_refs(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `const Foo(...)` — emit TypeRef/Calls for the constructed type.
            "const_object_expression" => {
                extract_const_object_refs(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // Legacy node names (kept for compatibility with older grammars or future use)
            "invocation_expression" | "function_invocation" => {
                let callee_node_opt = child
                    .child_by_field_name("function")
                    .or_else(|| child.child_by_field_name("name"));

                if let Some(callee_node) = callee_node_opt {
                    let chain = build_chain(callee_node, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| dart_callee_name(callee_node, src));

                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &callee_node, refs);
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // Dart grammar 0.1: function calls are `postfix_expression` with selector(s).
            // `bar()` → postfix_expression [identifier("bar"), selector(argument_part(arguments))]
            // `obj.bar()` → postfix_expression [identifier("obj"), selector(unconditional_assignable_selector), selector(argument_part)]
            "postfix_expression" => {
                extract_postfix_call(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // Alternative Dart call representation:
            // Dart grammar 0.1 often parses calls without a `postfix_expression` wrapper.
            // Instead, the `identifier` and `selector(argument_part(...))` appear as direct
            // siblings inside their container node.  This occurs in:
            //   expression_statement  — `bar();`
            //   initialized_variable_definition — `var d = Dog();`
            //   return_statement — `return f();`
            // Handle all of these uniformly.
            "expression_statement" | "initialized_variable_definition" | "return_statement" => {
                extract_inline_call_from_statement(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `new Dog(args)` — emit Calls edge to the constructed type.
            "new_expression" => {
                extract_new_expression_ref(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `Dog()` — implicit constructor invocation (no `new` keyword).
            // `constructor_invocation` has `type` field (type_identifier) + `arguments`.
            "constructor_invocation" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = match type_node.kind() {
                        "type_identifier" | "identifier" => node_text(type_node, src),
                        _ => {
                            let mut found = String::new();
                            let mut c = type_node.walk();
                            for inner in type_node.named_children(&mut c) {
                                if inner.kind() == "type_identifier" || inner.kind() == "identifier" {
                                    found = node_text(inner, src);
                                    break;
                                }
                            }
                            found
                        }
                    };
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `expr as Type` — emit TypeRef for the cast type.
            // Dart grammar 0.1 structure:
            //   type_cast_expression → [..., type_cast]
            //   type_cast            → ["as", type_identifier | function_type | ...]
            "type_cast_expression" => {
                // Find the `type_cast` child which holds the target type.
                let mut tc = child.walk();
                let mut emitted = false;
                for inner in child.named_children(&mut tc) {
                    if inner.kind() == "type_cast" {
                        // Walk type_cast for type_identifier.
                        let mut ic = inner.walk();
                        for grandchild in inner.named_children(&mut ic) {
                            if grandchild.kind() == "type_identifier" || grandchild.kind() == "identifier" {
                                emit_dart_type_ref(grandchild, src, source_symbol_index, refs);
                                emitted = true;
                                break;
                            }
                        }
                        break;
                    }
                }
                // Fallback: direct type_identifier in children
                if !emitted {
                    let mut tc2 = child.walk();
                    for inner in child.named_children(&mut tc2) {
                        if inner.kind() == "type_identifier" || inner.kind() == "identifier" {
                            emit_dart_type_ref(inner, src, source_symbol_index, refs);
                            break;
                        }
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `catch (e SpecificException)` — emit TypeRef for the exception type.
            "on_part" => {
                let mut oc = child.walk();
                for inner in child.named_children(&mut oc) {
                    if inner.kind() == "type_identifier" || inner.kind() == "identifier" {
                        emit_dart_type_ref(inner, src, source_symbol_index, refs);
                        break;
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // String interpolation
            "string_literal_double_quotes"
            | "string_literal_single_quotes"
            | "string_literal_double_quotes_multiple"
            | "string_literal_single_quotes_multiple" => {
                let mut sc = child.walk();
                for seg in child.named_children(&mut sc) {
                    if seg.kind() == "template_substitution" {
                        extract_dart_calls(&seg, src, source_symbol_index, refs);
                    }
                }
            }

            _ => {
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract a Calls ref from a `postfix_expression` that has an argument selector
/// (i.e. is an actual function/method invocation, not just a property access).
///
/// Dart grammar 0.1 structure:
///   `bar()`        → postfix_expression [ assignable_expression(identifier("bar")),
///                                         selector(argument_part(arguments)) ]
///   `obj.bar()`   → postfix_expression [ assignable_expression(identifier("obj")),
///                                         selector(unconditional_assignable_selector(".",identifier("bar"))),
///                                         selector(argument_part(arguments)) ]
fn extract_postfix_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Collect all direct children upfront to avoid borrow conflicts.
    let children: Vec<tree_sitter::Node> = {
        let mut c = node.walk();
        node.children(&mut c).collect()
    };

    // Check if any selector child contains an argument_part/arguments (= a function call).
    let has_call_selector = children.iter().any(|child| {
        if child.kind() == "selector" {
            let grandchildren: Vec<_> = {
                let mut sc = child.walk();
                child.children(&mut sc).collect::<Vec<_>>()
            };
            grandchildren.iter().any(|s| s.kind() == "argument_part" || s.kind() == "arguments")
        } else {
            false
        }
    });

    if !has_call_selector {
        return;
    }

    // Find the callee: last member name from non-argument selectors, or base identifier.
    let mut last_member: Option<String> = None;
    let mut callee_from_base: Option<String> = None;

    if let Some(base) = children.first() {
        // The base is typically `assignable_expression` wrapping an identifier.
        callee_from_base = ident_from_assignable(*base, src);
    }

    for child in children.iter().skip(1) {
        if child.kind() == "selector" {
            let selector_children: Vec<_> = {
                let mut sc = child.walk();
                child.children(&mut sc).collect()
            };
            for s in &selector_children {
                match s.kind() {
                    "unconditional_assignable_selector" | "conditional_assignable_selector" => {
                        let sub: Vec<_> = {
                            let mut uc = s.walk();
                            s.children(&mut uc).collect()
                        };
                        for u in &sub {
                            if u.kind() == "identifier" || u.kind() == "type_identifier" {
                                last_member = Some(node_text(*u, src));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let target = last_member.or(callee_from_base).unwrap_or_default();
    if !target.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

/// Handle the Dart grammar 0.1 pattern where a function call is represented as:
///   expression_statement [ identifier("bar"), selector(argument_part(arguments)) ]
/// instead of the expected postfix_expression wrapper.
///
/// This occurs for simple bare function calls like `bar()` and method calls like
/// `obj.method()` where the grammar places identifier + selector directly inside the
/// statement node without a postfix_expression wrapper.
/// Handle the Dart grammar 0.1 pattern where a function call is represented as:
///   container [ ..., identifier("callee"), selector(argument_part(arguments)), ... ]
/// instead of the expected postfix_expression wrapper.
///
/// Strategy: find the index of the first `selector(argument_part)` in the children list,
/// then take the last `identifier` or `type_identifier` that appears before that selector.
/// This correctly handles:
///   `bar()` → expression_statement(identifier("bar"), selector(...))
///   `var d = Dog()` → initialized_variable_definition(var, identifier("d"), =, identifier("Dog"), selector(...))
fn extract_inline_call_from_statement(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let children: Vec<tree_sitter::Node> = {
        let mut c = node.walk();
        node.children(&mut c).collect()
    };

    // Find index of first selector with argument_part (= the call site).
    let call_selector_idx = children.iter().position(|child| {
        if child.kind() == "selector" {
            let grandchildren: Vec<_> = {
                let mut sc = child.walk();
                child.children(&mut sc).collect::<Vec<_>>()
            };
            grandchildren.iter().any(|s| s.kind() == "argument_part" || s.kind() == "arguments")
        } else {
            false
        }
    });

    let call_idx = match call_selector_idx {
        Some(i) => i,
        None => return, // No function call selector found
    };

    // The callee: last identifier/type_identifier appearing before the call selector.
    // Also scan selector children for member access (obj.method()).
    let mut callee_ident: Option<String> = None;
    let mut last_member: Option<String> = None;

    // Scan children before the call selector for the last identifier.
    for child in &children[..call_idx] {
        match child.kind() {
            "identifier" | "type_identifier" => {
                callee_ident = Some(node_text(*child, src));
            }
            "assignable_expression" => {
                if let Some(name) = ident_from_assignable(*child, src) {
                    callee_ident = Some(name);
                }
            }
            "selector" => {
                // Non-argument selectors before the call selector = member access.
                let selector_children: Vec<_> = {
                    let mut sc = child.walk();
                    child.children(&mut sc).collect()
                };
                for s in &selector_children {
                    match s.kind() {
                        "unconditional_assignable_selector" | "conditional_assignable_selector" => {
                            let sub: Vec<_> = {
                                let mut uc = s.walk();
                                s.children(&mut uc).collect()
                            };
                            for u in &sub {
                                if u.kind() == "identifier" || u.kind() == "type_identifier" {
                                    last_member = Some(node_text(*u, src));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let target = last_member.or(callee_ident).unwrap_or_default();
    if !target.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

/// Extract the base identifier from an `assignable_expression` node (or plain identifier).
fn ident_from_assignable(node: tree_sitter::Node, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" => Some(node_text(node, src)),
        "assignable_expression" => {
            // Walk named children looking for an identifier.
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                match child.kind() {
                    "identifier" | "type_identifier" => return Some(node_text(child, src)),
                    _ => {}
                }
            }
            // Fallback: first named child recursion
            let mut c2 = node.walk();
            for child in node.named_children(&mut c2) {
                if let Some(name) = ident_from_assignable(child, src) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

/// Emit a Calls edge for `new Dog(args)`.
///
/// `new_expression` stores the type in the `type` field (a `type_identifier`)
/// and the arguments in the `arguments` field.  There are NO named children;
/// the type must be accessed via `child_by_field_name("type")`.
fn extract_new_expression_ref(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Try the `type` field first (Dart grammar 0.1).
    if let Some(type_node) = node.child_by_field_name("type") {
        let name = match type_node.kind() {
            "type_identifier" | "identifier" => node_text(type_node, src),
            _ => {
                // Walk into type_arguments → type_identifier
                let mut found = String::new();
                let mut c = type_node.walk();
                for child in type_node.named_children(&mut c) {
                    if child.kind() == "type_identifier" || child.kind() == "identifier" {
                        found = node_text(child, src);
                        break;
                    }
                }
                found
            }
        };
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
            return;
        }
    }
    // Fallback: walk all children for type_identifier.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" || child.kind() == "identifier" {
            let name = node_text(child, src);
            if !name.is_empty() && name != "new" {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
                return;
            }
        }
    }
}

fn dart_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "selector_expression" | "navigation_expression" => {
            if let Some(sel) = node.child_by_field_name("selector") {
                return node_text(sel, src);
            }
            let mut last = String::new();
            let mut c = node.walk();
            for n in node.children(&mut c) {
                if n.kind() == "identifier" || n.kind() == "simple_identifier" {
                    last = node_text(n, src);
                }
            }
            last
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

pub(super) fn build_chain(node: Node, src: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" | "simple_identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "super" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "selector_expression" => {
            let receiver = node
                .child_by_field_name("object")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;
            let member_name = node
                .child_by_field_name("selector")
                .map(|n| node_text(n, src))
                .or_else(|| {
                    let mut last: Option<String> = None;
                    let mut c = node.walk();
                    for child in node.children(&mut c) {
                        if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                            last = Some(node_text(child, src));
                        }
                    }
                    last
                })?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "selector_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "navigation_expression" => {
            let receiver = node
                .child_by_field_name("target")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;
            let mut last: Option<String> = None;
            let mut c = node.walk();
            for child in node.children(&mut c) {
                if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                    last = Some(node_text(child, src));
                }
            }
            let member_name = last?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "navigation_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "cascade_expression" => {
            let receiver = node.named_child(0)?;
            build_chain_inner(receiver, src, segments)
        }

        "invocation_expression" | "function_invocation" => {
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.named_child(0))?;
            build_chain_inner(callee, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Type reference helpers added for coverage gap fixes
// ---------------------------------------------------------------------------

/// Emit TypeRef edges for all type_identifier nodes inside a `type_arguments`
/// node (e.g. `List<MyModel>`, `Map<String, UserDto>`).
pub(super) fn extract_type_arguments_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                emit_dart_type_ref(child, src, source_symbol_index, refs);
            }
            // Recurse into nested type nodes (e.g. `Map<String, List<Foo>>`).
            "type_arguments" | "type_not_void" | "function_type" => {
                extract_type_arguments_refs(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_type_arguments_refs(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit TypeRef edges from a `type_test_expression` / `is_expression` node.
/// Dart: `x is MyType` — tree-sitter-dart 0.1 represents this as:
///   type_test_expression → [..., type_test]
///   type_test → ["is", type_not_void]
///   type_not_void → type_identifier | ...
pub(super) fn extract_type_test_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_test" => {
                let mut tc = child.walk();
                for inner in child.children(&mut tc) {
                    match inner.kind() {
                        "type_identifier" | "identifier" => {
                            emit_dart_type_ref(inner, src, source_symbol_index, refs);
                        }
                        "type_not_void" | "type_not_void_not_function" => {
                            // Walk into type_not_void for the type_identifier.
                            let mut vc = inner.walk();
                            for vchild in inner.children(&mut vc) {
                                if vchild.kind() == "type_identifier" || vchild.kind() == "identifier" {
                                    emit_dart_type_ref(vchild, src, source_symbol_index, refs);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "type_identifier" | "identifier" => {
                emit_dart_type_ref(child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

/// Emit TypeRef/Calls edges from a `const_object_expression` node.
/// Dart: `const Foo(...)` or `const package.Foo(...)`.
pub(super) fn extract_const_object_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk children for type_identifier (the class being constructed).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() && name != "const" {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                    return;
                }
            }
            _ => {}
        }
    }
}
