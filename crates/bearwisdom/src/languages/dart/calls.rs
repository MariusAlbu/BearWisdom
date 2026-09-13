// =============================================================================
// dart/calls.rs  —  Call extraction and member chain builder for Dart
// =============================================================================

pub(super) use super::call_args::extract_dart_call_args;
use super::call_sites::{extract_inline_call_from_statement, extract_postfix_call};
use super::helpers::node_text;
pub(super) use super::member_chain::{build_chain, dart_callee_name};
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;

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
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: type_node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: type_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

pub(super) fn extract_dart_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // An arrow body (`=> expr`) keeps its call as the body node's own children,
    // so the root itself is read as one statement before its children are
    // walked; a block body reaches its calls through statements.
    if matches!(node.kind(), "function_body" | "function_expression_body") {
        extract_inline_call_from_statement(node, src, source_symbol_index, refs);
    }
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

                    crate::languages::emit_chain_type_ref(
                        &chain,
                        source_symbol_index,
                        &callee_node,
                        refs,
                    );
                    if !target_name.is_empty() {
                        let call_args = extract_dart_call_args(&child, src);
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
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

            // Arrow-style closures keep their expression body directly under
            // `function_expression_body`, without an enclosing statement. The
            // grammar can surface its selector sequence as siblings instead of
            // a `postfix_expression`, so apply the same inline extraction once
            // before recursing. Block-bodied closures have no such siblings.
            "function_expression_body" => {
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // A call passed as an argument (`f(model.serialize())`) sits as
            // identifier + selector siblings directly under `argument`, with
            // no statement or `postfix_expression` around it.
            "argument" => {
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
                                if inner.kind() == "type_identifier" || inner.kind() == "identifier"
                                {
                                    found = node_text(inner, src);
                                    break;
                                }
                            }
                            found
                        }
                    };
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
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
                            if grandchild.kind() == "type_identifier"
                                || grandchild.kind() == "identifier"
                            {
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
                is_include: false,
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
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
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
                return;
            }
        }
    }
}

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
                                if vchild.kind() == "type_identifier"
                                    || vchild.kind() == "identifier"
                                {
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

/// Emit TypeRef/Instantiates edges from a `const_object_expression` node.
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
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Instantiates,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                    return;
                }
            }
            _ => {}
        }
    }
}
