// =============================================================================
// languages/erlang/functions.rs  —  fun_decl extraction and call collection
//
// Extracts top-level `fun_decl` nodes as Function symbols (with `name/arity`
// qualified names and exported/private visibility), and walks each function
// body to emit Calls / Instantiates references for `call`, `internal_fun`,
// `external_fun`, and `record_expr` nodes.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

use super::extract::node_text;

pub(super) fn extract_function(
    node: &Node,
    src: &str,
    exported: &std::collections::HashSet<String>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // fun_decl groups function_clause nodes
    // Get name from first function_clause → name field
    let name = get_function_name(node, src);
    if name.is_empty() {
        return;
    }

    // Compute arity from first clause argument count
    let arity = get_function_arity(node, src);
    let name_arity = format!("{}/{}", name, arity);
    let is_exported = exported.contains(&name_arity);

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name_arity.clone(),
        qualified_name: name_arity.clone(),
        kind: SymbolKind::Function,
        visibility: Some(if is_exported {
            Visibility::Public
        } else {
            Visibility::Private
        }),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("{}", name_arity)),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });

    // Extract calls inside function body
    collect_calls(node, src, idx, refs);
}

fn get_function_name(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_clause" {
            if let Some(name_node) = child.child_by_field_name("name") {
                return node_text(&name_node, src).to_string();
            }
            // Fallback: first identifier child
            let mut c2 = child.walk();
            for n in child.children(&mut c2) {
                if n.kind() == "atom" || n.kind() == "identifier" {
                    return node_text(&n, src).to_string();
                }
            }
        }
    }
    String::new()
}

fn get_function_arity(node: &Node, _src: &str) -> u32 {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_clause" {
            // Count argument nodes in `args` field
            if let Some(args) = child.child_by_field_name("args") {
                let count = args.child_count();
                // args typically wraps in parentheses; count non-punctuation children
                let non_punct = {
                    let mut c = args.walk();
                    args.children(&mut c)
                        .filter(|n| {
                            let k = n.kind();
                            k != "(" && k != ")" && k != ","
                        })
                        .count()
                };
                return if non_punct == 0 && count == 2 {
                    0
                } else {
                    non_punct as u32
                };
            }
            return 0;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Collect call edges from a subtree
// ---------------------------------------------------------------------------

/// Attribute names that look like calls but are module-level directives.
/// `-doc "..."`, `-moduledoc "..."`, etc. (OTP 27+) get parsed such that the
/// atom `doc` / `moduledoc` can appear as a call target.  Skip them.
const ATTR_CALL_SKIP: &[&str] = &[
    "doc",
    "moduledoc",
    "feature",
    "deprecated",
    "dialyzer",
    "nifs",
    "on_load",
    "compile",
    "vsn",
    "author",
];

/// Count the number of arguments in an `expr_args` node.
///
/// `expr_args` holds a `multiple: true` `args` field whose entries are the
/// individual argument expressions. tree-sitter represents multiple-field
/// nodes as direct named children of `expr_args`; they are mixed with
/// comma/paren grammar tokens that are anonymous (not named). Count only
/// named children — each one is exactly one argument.
fn count_expr_args(expr_args: &Node) -> u32 {
    let mut c = expr_args.walk();
    expr_args.children(&mut c).filter(|n| n.is_named()).count() as u32
}

/// Extract the integer text from an `arity` node.
///
/// An `arity` node in the grammar has the form `/N` where the leading slash
/// is anonymous punctuation. Its `value` field holds the integer node alone.
pub(super) fn arity_value<'a>(arity_node: &Node, src: &'a str) -> &'a str {
    if let Some(v) = arity_node.child_by_field_name("value") {
        node_text(&v, src)
    } else {
        // Fallback: strip leading slash if present in the raw text.
        node_text(arity_node, src).trim_start_matches('/')
    }
}

pub(super) fn collect_calls(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                // call.expr — function expression; call.args — expr_args with arguments.
                // Always emit at least one ref per `call` node so the coverage budget
                // is satisfied. For named calls (atom or remote), emit `name/arity` as
                // the target_name so the resolver can do exact arity-aware lookup.
                let call_line = child.start_position().row as u32;
                let arg_count = child
                    .child_by_field_name("args")
                    .map(|a| count_expr_args(&a))
                    .unwrap_or(0);

                let target = if let Some(expr) = child.child_by_field_name("expr") {
                    match expr.kind() {
                        "atom" => {
                            let name = node_text(&expr, src);
                            format!("{}/{}", name, arg_count)
                        }
                        "remote" => {
                            // Module:function call.
                            if let Some(fun_node) = expr.child_by_field_name("fun") {
                                let fun_name = node_text(&fun_node, src).to_string();
                                let module = expr
                                    .child_by_field_name("module")
                                    .map(|n| node_text(&n, src).to_string());
                                if !fun_name.is_empty() {
                                    refs.push(ExtractedRef {
                                        is_import_binding: false,
                                        is_reexport: false,
                                        source_symbol_index: source_idx,
                                        target_name: format!("{}/{}", fun_name, arg_count),
                                        kind: EdgeKind::Calls,
                                        line: call_line,
                                        col: 0,
                                        module,
                                        chain: None,
                                        byte_offset: child.start_byte() as u32,
                                        namespace_segments: Vec::new(),
                                        call_args: Vec::new(),
                                    });
                                }
                                String::new()
                            } else {
                                node_text(&expr, src).to_string()
                            }
                        }
                        _ => node_text(&expr, src).to_string(),
                    }
                } else {
                    // No `expr` field — use first named child as fallback (no arity suffix).
                    let mut fallback = String::new();
                    for ci in 0..child.child_count() {
                        if let Some(c) = child.child(ci) {
                            if c.is_named() {
                                fallback = node_text(&c, src).to_string();
                                break;
                            }
                        }
                    }
                    fallback
                };
                if !target.is_empty() {
                    // Strip arity suffix for the ATTR_CALL_SKIP check so that
                    // `doc/0` is still recognised as the `doc` directive.
                    let bare = target.split('/').next().unwrap_or(&target);
                    if !ATTR_CALL_SKIP.contains(&bare) {
                        refs.push(ExtractedRef {
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line: call_line,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
                collect_calls(&child, src, source_idx, refs);
            }
            "internal_fun" => {
                // fun foo/2 — explicit arity in `arity` field; emit "name/N".
                let line = child.start_position().row as u32;
                if let Some(fun_node) = child.child_by_field_name("fun") {
                    let name = node_text(&fun_node, src).to_string();
                    let arity = child
                        .child_by_field_name("arity")
                        .map(|n| arity_value(&n, src).to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let target = if arity.is_empty() {
                            name
                        } else {
                            format!("{}/{}", name, arity)
                        };
                        refs.push(ExtractedRef {
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                            col: 0,
                        });
                    }
                }
            }
            "external_fun" => {
                // fun mod:foo/2 — explicit arity in `arity` field; emit "name/N".
                let line = child.start_position().row as u32;
                if let Some(fun_node) = child.child_by_field_name("fun") {
                    let fun_name = node_text(&fun_node, src).to_string();
                    let arity = child
                        .child_by_field_name("arity")
                        .map(|n| arity_value(&n, src).to_string())
                        .unwrap_or_default();
                    let module = child
                        .child_by_field_name("module")
                        .map(|n| node_text(&n, src).to_string());
                    if !fun_name.is_empty() {
                        let target = if arity.is_empty() {
                            fun_name
                        } else {
                            format!("{}/{}", fun_name, arity)
                        };
                        refs.push(ExtractedRef {
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line,
                            module,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                            col: 0,
                        });
                    }
                }
            }
            "record_expr" => {
                // #record_name{...} — record construction
                let line = child.start_position().row as u32;
                if let Some(name_node) = child.child_by_field_name("name") {
                    // record_name has a `name` field itself
                    let record_name = if let Some(inner) = name_node.child_by_field_name("name") {
                        node_text(&inner, src).to_string()
                    } else {
                        node_text(&name_node, src).to_string()
                    };
                    if !record_name.is_empty() {
                        refs.push(ExtractedRef {
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: record_name,
                            kind: EdgeKind::Instantiates,
                            line,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                            col: 0,
                        });
                    }
                }
                collect_calls(&child, src, source_idx, refs);
            }
            _ => {
                collect_calls(&child, src, source_idx, refs);
            }
        }
    }
}
