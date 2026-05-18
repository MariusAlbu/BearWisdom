// =============================================================================
// languages/nix/calls.rs  —  apply_expression / with_expression / formal
// default extraction + call-name resolution helpers
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol};
use tree_sitter::Node;

use super::extract::{extract_value_refs, first_child_of_kind, first_identifier_text, is_expr_node, node_text};

// ---------------------------------------------------------------------------
// apply_expression  (function call / import)
// ---------------------------------------------------------------------------

pub(super) fn extract_apply(
    node: Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // apply_expression: function field + argument
    let func_node = node.child_by_field_name("function")
        .or_else(|| first_child_of_kind(&node, "variable_expression"))
        .or_else(|| {
            // First child that is an expression
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if is_expr_node(&child) {
                        return Some(child);
                    }
                }
            }
            None
        });

    let func_name = func_node.and_then(|n| {
        // For curried applies like `(f a) b`, the outer apply's function is another apply.
        // Recursively resolve to find the original function name.
        resolve_apply_func_name(n, src)
    });

    // If the function name can't be resolved (e.g. anonymous lambda `(x: ...)` in
    // function position, or a complex expression), use the node text as a fallback
    // target so coverage correlation can still match this apply site.
    let func_name = match func_name {
        Some(n) => n,
        None => {
            // Emit a minimal Calls ref with whatever text we can extract from the
            // function node, truncated to avoid noise. This ensures the apply site
            // registers a ref rather than being silently unmatched.
            let fallback = func_node.map(|n| {
                let t = node_text(n, src);
                // Limit to 80 chars to avoid giant lambda bodies as ref targets
                if t.len() > 80 { t[..80].to_string() } else { t }
            });
            if let Some(target) = fallback {
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: target,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            return;
        }
    };

    // `import` is a keyword/builtin in Nix — emit Imports edge when arg is a path.
    // If the arg is a complex expression (e.g. `nixpkgs + "/path"`), fall through
    // to emit a Calls edge for `import` itself so every apply site has a ref.
    if func_name == "import" {
        if let Some(arg) = apply_argument(&node) {
            if let Some(p) = extract_path_or_string(arg, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: p.clone(),
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: Some(p),
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
                return;
            }
        }
        // Path not extractable — fall through to emit Calls -> "import".
    }

    // `callPackage path {}` — emit Imports edge to the package path.
    // If the arg is not a literal path (unusual), fall through to a Calls edge.
    if func_name == "callPackage" || func_name.ends_with(".callPackage") {
        if let Some(arg) = apply_argument(&node) {
            if let Some(p) = extract_path_or_string(arg, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: p.clone(),
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: Some(p),
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
                return;
            }
        }
        // Path not extractable — fall through to emit Calls -> "callPackage".
    }

    // General function application — emit Calls edge.
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: func_name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Formal parameter defaults  ({ pkgs ? import <nixpkgs> { }, ... }: body)
// ---------------------------------------------------------------------------

/// Visit the default-value expressions of formal parameters in a
/// `function_expression` whose arguments are an attrset pattern.
///
/// Tree-sitter-nix represents formal parameters as children of the
/// `function_expression`'s first child (`formals` or `formal_set`).
/// Each `formal` node may have a default value after `?`.
///
/// These defaults are not reachable via body traversal, so we visit them
/// explicitly to capture apply_expressions like `import <nixpkgs> { }`.
pub(super) fn visit_formal_defaults(
    func_node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(symbols.len().saturating_sub(1));
    // The formals container is typically the first named child before `:`.
    let mut outer_cursor = func_node.walk();
    for child in func_node.children(&mut outer_cursor) {
        // Look for `formals`, `formal_set`, or `formal` nodes
        match child.kind() {
            "formals" | "formal_set" => {
                let mut fc = child.walk();
                for formal in child.children(&mut fc) {
                    if formal.kind() == "formal" {
                        visit_formal_default(formal, src, source_idx, symbols, refs);
                    }
                }
            }
            "formal" => {
                visit_formal_default(child, src, source_idx, symbols, refs);
            }
            _ => {}
        }
    }
}

fn visit_formal_default(
    formal: Node,
    src: &str,
    source_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // formal: identifier ? default_expr
    // The default expression is the expression child after `?`
    let mut cursor = formal.walk();
    let mut past_question_mark = false;
    for child in formal.children(&mut cursor) {
        if !child.is_named() && node_text(child, src) == "?" {
            past_question_mark = true;
            continue;
        }
        if past_question_mark && is_expr_node(&child) {
            extract_value_refs(child, src, source_idx, symbols, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// with_expression  (with pkgs; ...)
// ---------------------------------------------------------------------------

pub(super) fn extract_with(
    node: Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // with_expression: environment field is the namespace being brought in scope
    let env = node.child_by_field_name("environment").or_else(|| {
        // First expression child (the namespace before `;`)
        let mut cursor = node.walk();
        let found: Option<Node> = {
            let mut iter = node.children(&mut cursor);
            iter.find(|c| is_expr_node(c))
        };
        found
    });

    if let Some(env_node) = env {
        if let Some(name) = resolve_var_name(env_node, src) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
    }
}

// ---------------------------------------------------------------------------
// Call name resolution
// ---------------------------------------------------------------------------

/// Resolve the ultimate function name from the function position of an apply_expression.
/// Handles curried applies: `(f a) b` → `f a` is another apply → resolve recursively.
fn resolve_apply_func_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "variable_expression" | "identifier" | "select_expression" => resolve_call_name(node, src),
        "apply_expression" => {
            // Curried call: resolve the inner function
            let inner_func = node.child_by_field_name("function").or_else(|| {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if is_expr_node(&child) {
                            return Some(child);
                        }
                    }
                }
                None
            });
            inner_func.and_then(|n| resolve_apply_func_name(n, src))
        }
        "parenthesized_expression" => {
            // The inner expression is the actual function — recurse into it.
            // E.g. `(builtins.fetchTarball url) {}` has a parenthesized apply as func.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    if let Some(name) = resolve_apply_func_name(child, src) {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => resolve_call_name(node, src),
    }
}

pub(super) fn resolve_call_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "variable_expression" => {
            node.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .or_else(|| first_identifier_text(&node, src))
        }
        "identifier" => Some(node_text(node, src)),
        "select_expression" => {
            // e.g., `lib.makeOverridable` or `pkgs.stdenv`
            // Build the full dotted path
            let mut parts = Vec::new();
            collect_select_path(node, src, &mut parts);
            if parts.is_empty() { None } else { Some(parts.join(".")) }
        }
        _ => None,
    }
}

fn collect_select_path(node: Node, src: &str, parts: &mut Vec<String>) {
    match node.kind() {
        "select_expression" => {
            // select_expression: expression.attrpath
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "variable_expression" | "identifier" => {
                        if let Some(n) = first_identifier_text(&child, src)
                            .or_else(|| {
                                child.child_by_field_name("name")
                                    .map(|n| node_text(n, src))
                            })
                        {
                            parts.push(n);
                        }
                    }
                    "select_expression" => collect_select_path(child, src, parts),
                    "attrpath" | "attr" | "identifier" => {
                        parts.push(node_text(child, src));
                    }
                    _ => {}
                }
            }
        }
        "variable_expression" => {
            if let Some(n) = first_identifier_text(&node, src) {
                parts.push(n);
            }
        }
        "identifier" => {
            parts.push(node_text(node, src));
        }
        _ => {}
    }
}

pub(super) fn resolve_var_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "variable_expression" => {
            node.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .or_else(|| first_identifier_text(&node, src))
        }
        "identifier" => Some(node_text(node, src)),
        _ => first_identifier_text(&node, src),
    }
}

fn apply_argument<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    // The argument in apply_expression is the second expression child
    if let Some(arg) = node.child_by_field_name("argument") {
        return Some(arg);
    }
    // Fallback: second named expression child
    let mut cursor = node.walk();
    let mut count = 0usize;
    for child in node.children(&mut cursor) {
        if is_expr_node(&child) {
            if count == 1 {
                return Some(child);
            }
            count += 1;
        }
    }
    None
}

fn extract_path_or_string(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "path_expression" | "hpath_expression" | "spath_expression" => {
            Some(node_text(node, src))
        }
        "string_expression" | "indented_string_expression" => {
            // Strip quotes
            let raw = node_text(node, src);
            Some(raw.trim_matches('"').trim_matches('\'').to_string())
        }
        _ => {
            // Recurse into parenthesized
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(p) = extract_path_or_string(child, src) {
                    return Some(p);
                }
            }
            None
        }
    }
}
