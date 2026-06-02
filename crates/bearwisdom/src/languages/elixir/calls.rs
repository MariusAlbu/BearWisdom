// =============================================================================
// Call extraction inside Elixir function bodies
//
// Emits Calls edges for invocations encountered in expression positions —
// direct calls, `Mod.fun(...)` dot calls, and `|>` pipe chains. Recurses
// through nested do-blocks and anonymous-fn bodies.
// =============================================================================

use super::helpers::{call_identifier, call_qualified_name, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

pub(super) fn extract_calls_recursive(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                if let Some(callee) = call_identifier(&child, src) {
                    if !matches!(callee.as_str(), "def" | "defp" | "defmacro" | "defmacrop" | "defmodule" | "defstruct" | "defexception" | "defprotocol" | "defimpl" | "defguard" | "defguardp" | "alias" | "import" | "use" | "require") {
                        // Use the qualified form so dotted calls carry the
                        // module prefix through to the resolver.
                        let qualified = call_qualified_name(&child, src)
                            .unwrap_or_else(|| callee.clone());
                        let simple = qualified.rsplit('.').next().unwrap_or(&qualified).to_string();
                        let module = qualified.rfind('.').map(|i| qualified[..i].to_string());
                        // Lowercase dot-call receivers (`session.acquisition_channel`,
                        // `email.html_body`) are struct/map field access, not function
                        // calls. tree-sitter-elixir parses the form as `call` even
                        // without parens, but no real function lookup applies. Skip
                        // emission to keep the unresolved table clean.
                        let receiver_is_module = match &module {
                            Some(m) => m.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                                || m.contains('.'),
                            None => true, // bare call — keep
                        };
                        if receiver_is_module {
                            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                                source_symbol_index,
                                target_name: simple,
                                kind: EdgeKind::Calls,
                                line: child.start_position().row as u32,
                                col: 0,
                                module,
                                chain: None,
                                byte_offset: child.start_byte() as u32,
                                                        namespace_segments: Vec::new(),
                                                        call_args: Vec::new(),
});
                            // For dot calls like `Enum.map(...)`, also emit a TypeRef
                            // for the module part (the `alias` node before the dot).
                            extract_dot_call_module_ref(&child, src, source_symbol_index, refs);
                        }
                    }
                }
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }

            // Pipe operator: `value |> function_name(args)`
            "binary_operator" => {
                extract_pipe_calls(&child, src, source_symbol_index, refs);
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }

            // `alias` node (a capitalized module reference like `Enum`, `MyApp.User`).
            // Emit a TypeRef so that module references in expressions are tracked.
            "alias" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: if name.contains('.') { Some(name) } else { None },
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }

            // Anonymous functions: `fn arg -> body end` — recurse into stab_clause bodies.
            "anonymous_function" => {
                let mut fc = child.walk();
                for clause in child.children(&mut fc) {
                    if clause.kind() == "stab_clause" {
                        if let Some(body) = clause.child_by_field_name("body") {
                            extract_calls_recursive(&body, src, source_symbol_index, refs);
                        } else {
                            // fallback: last named child of stab_clause is the body
                            let clause_children: Vec<_> = {
                                let mut cc = clause.walk();
                                clause.named_children(&mut cc).collect()
                            };
                            if let Some(last) = clause_children.last() {
                                extract_calls_recursive(last, src, source_symbol_index, refs);
                            }
                        }
                    }
                }
            }

            // Keyword lists, maps, tuples, lists can contain calls — recurse.
            "keywords" | "keyword_list" | "map" | "tuple" | "list"
            | "arguments" | "body" | "block" | "do_block"
            | "access_call" | "unary_operator" => {
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// For a dot call like `Enum.map(...)`, emit a TypeRef to the receiver module.
///
/// The tree-sitter-elixir `call` node for `Enum.map(...)` has a `dot` child
/// whose first named child is the module (`alias` or `identifier`).
///
/// Only emits when the receiver looks like a module — uppercase first char or
/// dotted path. Lowercase receivers (`conn.cookies`, `assigns.user`) are
/// struct/map values, not modules; emitting them as TypeRef floods the
/// unresolved table with ~8000 lowercase locals on a Phoenix codebase.
pub(super) fn extract_dot_call_module_ref(
    call_node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        if child.kind() == "dot" {
            // `dot` → receiver (alias/identifier) . function_name
            let mut dc = child.walk();
            for dc_child in child.children(&mut dc) {
                match dc_child.kind() {
                    "alias" | "identifier" => {
                        let name = node_text(dc_child, src);
                        if !name.is_empty() {
                            let first_char = name.chars().next().unwrap_or('_');
                            if !(first_char.is_uppercase() || name.contains('.')) {
                                return;
                            }
                            let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                                source_symbol_index,
                                target_name: simple,
                                kind: EdgeKind::TypeRef,
                                line: dc_child.start_position().row as u32,
                                col: 0,
                                module: if name.contains('.') { Some(name) } else { None },
                                chain: None,
                                byte_offset: dc_child.start_byte() as u32,
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
});
                        }
                        return; // only the receiver, not the function name
                    }
                    _ => {}
                }
            }
            return;
        }
    }
}

/// Emit a Calls edge for the right-hand side of a `|>` pipe expression.
pub(super) fn extract_pipe_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Collect all children to find the operator and operands.
    let children: Vec<tree_sitter::Node> = {
        let mut cursor = node.walk();
        node.children(&mut cursor).collect()
    };

    // Find the `|>` operator position.
    let pipe_pos = match children.iter().position(|c| node_text(*c, src) == "|>") {
        Some(p) => p,
        None => return, // not a pipe expression
    };

    // The right operand is the child after `|>`.
    let right = match children.get(pipe_pos + 1) {
        Some(r) => r,
        // Also try field name as a fallback.
        None => match node.child_by_field_name("right") {
            Some(r) => {
                let name = extract_pipe_callee_name(&r, src);
                if let Some(n) = name {
                    if !n.is_empty() {
                        let simple = n.rsplit('.').next().unwrap_or(&n).to_string();
                        let module = n.rfind('.').map(|i| n[..i].to_string());
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name: simple,
                            kind: EdgeKind::Calls,
                            line: r.start_position().row as u32,
                            col: 0,
                            module,
                            chain: None,
                            byte_offset: r.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                        // Also emit TypeRef for module part of dot calls on the right side.
                        extract_dot_call_module_ref(&r, src, source_symbol_index, refs);
                    }
                }
                return;
            }
            None => return,
        },
    };

    let name = extract_pipe_callee_name(right, src);
    if let Some(n) = name {
        if !n.is_empty() {
            let simple = n.rsplit('.').next().unwrap_or(&n).to_string();
            let module = n.rfind('.').map(|i| n[..i].to_string());
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index,
                target_name: simple,
                kind: EdgeKind::Calls,
                line: right.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: right.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
            // Also emit TypeRef for module part of dot calls (`Enum.map`, etc.).
            extract_dot_call_module_ref(right, src, source_symbol_index, refs);
        }
    }
}

/// Extract the function name from the right side of a `|>` pipe.
///
/// Handles:
///   `validate(record)`       → "validate"   (identifier call)
///   `Enum.map(fn ...)`       → "map"         (dot-access call)
///   `String.length`          → "length"      (dot access, no parens)
///   `&String.upcase/1`       → "upcase"      (capture expression)
///   `&validate/1`            → "validate"    (bare capture)
fn extract_pipe_callee_name(node: &Node, src: &str) -> Option<String> {
    match node.kind() {
        "call" => {
            // Check if this is a dot-access call: `Enum.map(...)`
            // The call's first child is a `dot` node for qualified calls.
            // Return the FULL dotted form (`Module.func`) so the caller
            // can split off `module` for the resolver. Without the
            // module qualifier, the resolver only sees `func` and can't
            // disambiguate between `DateTime.utc_now` / `NaiveDateTime.utc_now`
            // / `Date.utc_today`, etc.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dot" => {
                        // `dot` → alias/identifier . identifier
                        // Reconstruct `module.tail` from text so the
                        // module prefix flows through.
                        let text = node_text(child, src);
                        if !text.is_empty() {
                            return Some(text);
                        }
                        // Fallback: walk children for the last ident
                        let mut dc = child.walk();
                        let mut last_ident: Option<String> = None;
                        for dc_child in child.children(&mut dc) {
                            if dc_child.kind() == "identifier" {
                                last_ident = Some(node_text(dc_child, src));
                            }
                        }
                        return last_ident;
                    }
                    "identifier" => {
                        // Bare call: `validate(...)`
                        return Some(node_text(child, src));
                    }
                    "alias" => {
                        // Module reference — use it as-is (shouldn't be a bare pipe target)
                        return Some(node_text(child, src));
                    }
                    _ => {}
                }
            }
            // Fallback to call_identifier
            call_identifier(node, src)
        }
        "identifier" => Some(node_text(*node, src)),
        "alias" => Some(node_text(*node, src)),
        // Capture expressions: `&String.upcase/1`, `&validate/1`
        // tree-sitter: unary_operator("&", binary_operator(dot_or_ident, "/", integer))
        "unary_operator" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "binary_operator" {
                    // The left side of the `/` is the function reference.
                    let children: Vec<_> = {
                        let mut c = child.walk();
                        child.children(&mut c).collect()
                    };
                    // Find `/` operator, take left side.
                    if let Some(slash_pos) = children.iter().position(|c| node_text(*c, src) == "/") {
                        if let Some(left) = children.get(slash_pos.saturating_sub(1)) {
                            // `left` may be a `call` (dot call) or `identifier` or `dot`.
                            return extract_pipe_callee_name(left, src);
                        }
                    }
                    // No slash — treat the whole expression as the name source.
                    return extract_pipe_callee_name(&child, src);
                }
                // `&identifier` without arity
                if child.kind() == "identifier" {
                    return Some(node_text(child, src));
                }
                if child.kind() == "call" || child.kind() == "dot" {
                    return extract_pipe_callee_name(&child, src);
                }
            }
            None
        }
        // `dot` node directly (qualified access without parens): `String.upcase`
        "dot" => {
            let mut cursor = node.walk();
            let mut last_ident: Option<String> = None;
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    last_ident = Some(node_text(child, src));
                }
            }
            last_ident
        }
        // Another binary_operator on the right side of a pipe: chained pipe expressions
        // that tree-sitter may represent as nested binary_operators.
        // Extract from the right operand of the nested pipe.
        "binary_operator" => {
            // If this binary_operator is itself a `|>`, extract the rightmost callee.
            let children: Vec<tree_sitter::Node> = {
                let mut c = node.walk();
                node.children(&mut c).collect()
            };
            if let Some(pipe_pos) = children.iter().position(|c| node_text(*c, src) == "|>") {
                if let Some(right) = children.get(pipe_pos + 1) {
                    return extract_pipe_callee_name(right, src);
                }
            }
            None
        }
        // Parenthesized expression: `(Module.fun)` — unwrap and recurse.
        "parenthesized_expression" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let result = extract_pipe_callee_name(&child, src);
                    if result.is_some() {
                        return result;
                    }
                }
            }
            None
        }
        _ => None,
    }
}
