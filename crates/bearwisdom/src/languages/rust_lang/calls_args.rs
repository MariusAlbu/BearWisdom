// =============================================================================
// rust/calls_args.rs  —  Argument extraction for call_expression / macro_invocation
// =============================================================================

use super::helpers::node_text;
use crate::types::CallArg;
use tree_sitter::Node;

#[cfg(test)]
#[path = "calls_args_tests.rs"]
mod tests;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 32;

// ---------------------------------------------------------------------------
// Argument extraction for call_expression / macro_invocation
// ---------------------------------------------------------------------------

/// Extract positional arguments from a Rust `arguments` node attached to a
/// `call_expression`. Captures string literals (including raw strings),
/// identifiers, scoped identifiers (`User::ID`), numeric and boolean
/// literals, and the recursive expression shapes (array, await, index,
/// binary). Anything else is `CallArg::Other`. Used by `detect_flow_emission`
/// to read URL strings and entity names off chain calls.
pub(super) fn extract_call_args(call_node: &Node, source: &str) -> Vec<CallArg> {
    let Some(args_node) = call_node.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        out.push(extract_arg(&child, source, 0));
    }
    out
}

/// Convert a single Rust expression node to a `CallArg`, recursing for
/// composite expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, source: &str, depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string_literal" | "raw_string_literal" => {
            CallArg::StringLit(strip_rust_string(&node_text(node, source)))
        }
        "identifier" => CallArg::Ident(node_text(node, source)),
        "scoped_identifier" => {
            let raw = node_text(node, source);
            let simple = raw.rsplit("::").next().unwrap_or(&raw).to_string();
            CallArg::Ident(simple)
        }
        // `&request` / `&mut buf` — passing references.
        "reference_expression" => {
            let inner = (0..node.named_child_count()).find_map(|i| node.named_child(i));
            match inner {
                Some(n) if n.kind() == "identifier" => CallArg::Ident(node_text(&n, source)),
                Some(n) if n.kind() == "scoped_identifier" => {
                    let raw = node_text(&n, source);
                    CallArg::Ident(raw.rsplit("::").next().unwrap_or(&raw).to_string())
                }
                _ => CallArg::Other,
            }
        }
        "integer_literal" | "float_literal" => CallArg::Literal(node_text(node, source)),
        "boolean_literal" => CallArg::Literal(node_text(node, source)),
        // `[a, b, c]` — recurse on each element. The optional `length`
        // child of the `[expr; N]` form is also a named child, so it
        // recurses too; it collapses to a harmless leaf variant.
        "array_expression" => {
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .map(|c| extract_arg(&c, source, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }
        // `expr.await` — single named child is the awaited expression.
        "await_expression" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, source, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Await { expr: Box::new(inner) }
        }
        // `container[index]` — two unnamed-field children in order.
        "index_expression" => {
            let container = node
                .named_child(0)
                .map(|n| extract_arg(&n, source, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .named_child(1)
                .map(|n| extract_arg(&n, source, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }
        // `left op right` — capture operator text and recurse on operands.
        "binary_expression" => {
            let op = node
                .child_by_field_name("operator")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let left = node
                .child_by_field_name("left")
                .map(|n| extract_arg(&n, source, depth + 1))
                .unwrap_or(CallArg::Other);
            let right = node
                .child_by_field_name("right")
                .map(|n| extract_arg(&n, source, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }
        // Nested call: capture the callee identifier so axum-style
        // `route("/x", get(handler))` exposes the HTTP verb to the
        // detector via `call_args[1]`.
        "call_expression" => {
            if let Some(func) = node.child_by_field_name("function") {
                let raw = node_text(&func, source);
                let simple = raw
                    .rsplit("::")
                    .next()
                    .unwrap_or(&raw)
                    .rsplit('.')
                    .next()
                    .unwrap_or(&raw)
                    .trim()
                    .to_string();
                if !simple.is_empty() {
                    CallArg::Ident(simple)
                } else {
                    CallArg::Other
                }
            } else {
                CallArg::Other
            }
        }
        _ => CallArg::Other,
    }
}

/// Extract the first string-literal-shaped argument from a `macro_invocation`
/// token_tree. Used for SQLx `query!("SELECT ...")` style macros where the
/// macro body is opaque and not parsed into structured `arguments`.
pub(super) fn extract_macro_string_args(macro_node: &Node, source: &str) -> Vec<CallArg> {
    let mut tt_node: Option<Node> = None;
    let mut walker = macro_node.walk();
    for c in macro_node.children(&mut walker) {
        let kind = c.kind();
        if kind == "token_tree" || kind.contains("token_tree") {
            tt_node = Some(c);
        }
    }
    let Some(tt) = tt_node else { return Vec::new() };
    let tt_text = node_text(&tt, source);
    if tt_text.len() < 2 {
        return Vec::new();
    }
    let inner = &tt_text[1..tt_text.len() - 1];
    let mut out = Vec::new();
    // First positional: try identifier (e.g. `query_as!(User, "SELECT ...")`).
    let trimmed = inner.trim_start();
    let mut after_first_ident: Option<&str> = None;
    if let Some(first) = trimmed
        .split(|c: char| c == ',' || c == '\n')
        .next()
        .map(|s| s.trim())
    {
        if !first.is_empty()
            && first.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
            && first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        {
            // Bare identifier or scoped identifier — capture last segment.
            let simple = first.rsplit("::").next().unwrap_or(first).to_string();
            out.push(CallArg::Ident(simple));
            if let Some(comma) = inner.find(',') {
                after_first_ident = Some(&inner[comma + 1..]);
            }
        }
    }
    // Scan for the first `"..."` or `r"..."` / `r#"..."#` style string.
    let scan = after_first_ident.unwrap_or(inner);
    if let Some(s) = find_first_rust_string_literal(scan) {
        out.push(CallArg::StringLit(s));
    }
    out
}

fn find_first_rust_string_literal(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // r#"..."# or r"..."
        if c == b'r' && i + 1 < bytes.len() {
            let mut hashes = 0;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let start = j + 1;
                let mut k = start;
                while k < bytes.len() {
                    if bytes[k] == b'"' {
                        let mut hash_match = 0;
                        let mut m = k + 1;
                        while hash_match < hashes && m < bytes.len() && bytes[m] == b'#' {
                            hash_match += 1;
                            m += 1;
                        }
                        if hash_match == hashes {
                            return Some(String::from_utf8_lossy(&bytes[start..k]).to_string());
                        }
                    }
                    k += 1;
                }
                return None;
            }
        }
        if c == b'"' {
            let start = i + 1;
            let mut k = start;
            while k < bytes.len() {
                if bytes[k] == b'\\' {
                    k += 2;
                    continue;
                }
                if bytes[k] == b'"' {
                    return Some(String::from_utf8_lossy(&bytes[start..k]).to_string());
                }
                k += 1;
            }
            return None;
        }
        i += 1;
    }
    None
}

fn strip_rust_string(raw: &str) -> String {
    // `"x"` → x.  `r"x"` / `r#"x"#` → x.
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix('r') {
        let trimmed = rest.trim_matches('#');
        let inner = trimmed.trim_start_matches('"').trim_end_matches('"');
        return inner.to_string();
    }
    s.trim_start_matches('"').trim_end_matches('"').to_string()
}
