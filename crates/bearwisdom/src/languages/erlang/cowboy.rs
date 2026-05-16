// =============================================================================
// languages/erlang/cowboy.rs  —  Cowboy router route extraction
//
// Erlang Cowboy declares HTTP routes through a single setup call:
//
//   Dispatch = cowboy_router:compile([
//       {'_', [
//           {"/users", users_handler, []},
//           {"/users/:id", user_handler, []}
//       ]}
//   ]).
//
// Each inner triple `{Path, HandlerModule, InitArgs}` is one route. Cowboy
// dispatches all HTTP methods to the handler's `init/2` callback, so we
// record `http_method = ""` (Any) and let the pair matcher key on the URL.
// =============================================================================

use crate::types::{ExtractedRoute, ExtractedSymbol};
use tree_sitter::Node;

use super::extract::node_text;

pub(crate) fn scan_cowboy_routes(
    root: Node,
    src: &str,
    symbols: &[ExtractedSymbol],
    routes: &mut Vec<ExtractedRoute>,
) {
    // Two-track detection. The AST walker is preferred because it lets us
    // attribute the route to the right source line; if the grammar shape
    // changes and the walker misses a call, the text-fallback below
    // catches the routes anyway. Tracks `seen_at` byte offsets so the same
    // dispatch table isn't recorded twice when both tracks fire.
    let count_before = routes.len();
    visit_for_cowboy(&root, src, symbols, routes);
    if routes.len() > count_before {
        return;
    }
    // Text fallback — `cowboy_router:compile(...)` literal lookup.
    let needle = "cowboy_router:compile(";
    let mut start = 0usize;
    while let Some(rel) = src[start..].find(needle) {
        let pos = start + rel + needle.len();
        // pos points just after the `(`. Find the matching `)` to bound the
        // argument text, then parse triples inside it.
        if let Some(end) = find_matching_paren(src, pos - 1) {
            let inner = &src[pos..end];
            extract_cowboy_triples_from_text(inner, routes, symbols, 1);
            start = end + 1;
        } else {
            start = pos;
        }
    }
}

fn find_matching_paren(text: &str, open_idx: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if open_idx >= bytes.len() || bytes[open_idx] != b'(' {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' => {
                i = skip_string_literal(text, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn visit_for_cowboy(
    node: &Node,
    src: &str,
    symbols: &[ExtractedSymbol],
    routes: &mut Vec<ExtractedRoute>,
) {
    if node.kind() == "call" {
        if is_cowboy_compile_call(node, src) {
            // Capture from the args field. Walk the source text inside the
            // outermost square-bracketed list and extract triples.
            if let Some(args_node) = node.child_by_field_name("args") {
                let args_text = node_text(&args_node, src);
                extract_cowboy_triples_from_text(args_text, routes, symbols, node.start_position().row as u32 + 1);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_for_cowboy(&child, src, symbols, routes);
    }
}

fn is_cowboy_compile_call(node: &Node, src: &str) -> bool {
    let Some(expr) = node.child_by_field_name("expr") else { return false };
    if expr.kind() != "remote" {
        return false;
    }
    let module = expr
        .child_by_field_name("module")
        .map(|n| node_text(&n, src))
        .unwrap_or("");
    let fun = expr
        .child_by_field_name("fun")
        .map(|n| node_text(&n, src))
        .unwrap_or("");
    module == "cowboy_router" && fun == "compile"
}

/// Parse `[{Host, [{Path, Handler, _}, ...]}, ...]` from raw source text.
/// We brace-match `{...}` tuples and recognise `{Path, Handler, ...}` shape:
/// first child a `"..."` string starting with `/`, second child an atom.
pub(crate) fn extract_cowboy_triples_from_text(
    text: &str,
    routes: &mut Vec<ExtractedRoute>,
    symbols: &[ExtractedSymbol],
    fallback_line: u32,
) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Find matching closing brace, honoring string literals.
            let end = match find_matching_brace(text, i) {
                Some(e) => e,
                None => break,
            };
            let inner = &text[i + 1..end];
            if let Some(route) = parse_cowboy_triple(inner, fallback_line, symbols) {
                routes.push(route);
            } else {
                // Nested tuples — recurse into inner.
                extract_cowboy_triples_from_text(inner, routes, symbols, fallback_line);
            }
            i = end + 1;
        } else if bytes[i] == b'"' {
            // Skip over string literal contents.
            i = skip_string_literal(text, i);
        } else {
            i += 1;
        }
    }
}

/// Given the inside of `{...}`, return Some(route) if it parses as a
/// Cowboy route triple `{Path, HandlerAtom, _}`. Otherwise None.
fn parse_cowboy_triple(
    inner: &str,
    fallback_line: u32,
    symbols: &[ExtractedSymbol],
) -> Option<ExtractedRoute> {
    let parts = split_top_level_commas(inner);
    if parts.len() < 2 {
        return None;
    }
    let first = parts[0].trim();
    let second = parts[1].trim();
    // First arg must be a string literal starting with `"/`.
    let path = if first.starts_with('"') && first.ends_with('"') && first.len() >= 2 {
        let raw = &first[1..first.len() - 1];
        if !raw.starts_with('/') {
            return None;
        }
        normalize_cowboy_path(raw)
    } else {
        return None;
    };
    // Second arg must be an atom (lowercase identifier).
    let is_atom = second
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_lowercase() || c == '_')
        && second
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@');
    if !is_atom {
        return None;
    }
    let handler_symbol_index = symbols
        .iter()
        .position(|s| s.start_line == fallback_line)
        .unwrap_or(0);
    Some(ExtractedRoute {
        handler_symbol_index,
        http_method: String::new(),
        template: path,
    })
}

/// Convert Cowboy's `:name` bindings to `{name}` so the URL normalizer
/// aligns the path with Producer-side `/users/{id}` strings.
fn normalize_cowboy_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' {
            // Bind variable — consume identifier chars.
            let mut name = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_ascii_alphanumeric() || nc == '_' {
                    name.push(nc);
                    chars.next();
                } else {
                    break;
                }
            }
            if name.is_empty() {
                out.push(':');
            } else {
                out.push('{');
                out.push_str(&name);
                out.push('}');
            }
        } else if c == '[' {
            // Drop optional-segment brackets — `/users[/:id]` → `/users/:id`.
            // Cowboy uses `[]` for optional path segments.
        } else if c == ']' {
            // Same — drop.
        } else {
            out.push(c);
        }
    }
    out
}

/// Find the byte index of the `}` that closes the `{` at `open_idx`. Honors
/// nested braces and skips string-literal contents.
fn find_matching_brace(text: &str, open_idx: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    debug_assert_eq!(bytes[open_idx], b'{');
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' => {
                i = skip_string_literal(text, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_string_literal(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Split a comma-separated argument list while respecting nested
/// `{}`, `[]`, `()`, and `""`.
fn split_top_level_commas(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth_brace = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_paren = 0i32;
    let mut in_string = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            current.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                current.push(c);
            }
            '{' => {
                depth_brace += 1;
                current.push(c);
            }
            '}' => {
                depth_brace -= 1;
                current.push(c);
            }
            '[' => {
                depth_bracket += 1;
                current.push(c);
            }
            ']' => {
                depth_bracket -= 1;
                current.push(c);
            }
            '(' => {
                depth_paren += 1;
                current.push(c);
            }
            ')' => {
                depth_paren -= 1;
                current.push(c);
            }
            ',' if depth_brace == 0 && depth_bracket == 0 && depth_paren == 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
