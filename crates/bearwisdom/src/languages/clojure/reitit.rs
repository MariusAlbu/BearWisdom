// Reitit data-driven route extraction.
//
// Reitit declares HTTP routes as plain Clojure data:
//
//   (def routes
//     ["/api"
//      ["/users" {:get list-users
//                 :post create-user}]
//      ["/users/:id" {:get user-by-id
//                     :delete delete-user}]])
//
// Each route is a vec_lit whose first element is a string literal starting
// with `/`. Subsequent elements are either nested route vec_lits
// (children with a relative path prefix) or a map_lit keyed by `:get`,
// `:post`, `:put`, `:patch`, `:delete`, `:head`, `:options`, or `:any`.
//
// The Compojure macro form (`(GET "/x" [] handler)`) is handled by the
// resolver in `resolve.rs::detect_clj_compojure_route`; this scanner
// focuses on the data form.

use crate::types::{ExtractedRoute, ExtractedSymbol};
use tree_sitter::Node;

use super::scope::sym_lit_name;

pub(super) fn scan_reitit_routes(
    node: Node,
    src: &[u8],
    parent_path: &str,
    symbols: &[ExtractedSymbol],
    routes: &mut Vec<ExtractedRoute>,
) {
    if node.kind() == "vec_lit" {
        if let Some(path_seg) = first_string_child(node, src) {
            let combined = combine_reitit_paths(parent_path, &path_seg);
            // Walk the remaining children: nested vec_lits inherit `combined`;
            // map_lits emit one route per HTTP-verb key.
            let mut cursor = node.walk();
            let mut saw_first_string = false;
            for child in node.children(&mut cursor) {
                if !saw_first_string {
                    if child.kind() == "str_lit" {
                        saw_first_string = true;
                    }
                    continue;
                }
                match child.kind() {
                    "vec_lit" => {
                        scan_reitit_routes(child, src, &combined, symbols, routes);
                    }
                    "map_lit" => {
                        scan_reitit_method_map(child, src, &combined, symbols, routes);
                    }
                    _ => {}
                }
            }
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_reitit_routes(child, src, parent_path, symbols, routes);
    }
}

fn scan_reitit_method_map(
    map_node: Node,
    src: &[u8],
    template: &str,
    symbols: &[ExtractedSymbol],
    routes: &mut Vec<ExtractedRoute>,
) {
    // map_lit children alternate: key, value, key, value, ... (kwd_lit + body).
    let mut pending_method: Option<String> = None;
    let mut cursor = map_node.walk();
    for child in map_node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        if pending_method.is_none() {
            if child.kind() == "kwd_lit" {
                let text = node_text_str(child, src);
                let kw = text.trim_start_matches(':');
                if let Some(method) = reitit_verb_to_method(kw) {
                    pending_method = Some(method.to_string());
                } else {
                    // Non-verb key (`:middleware`, `:name`, `:summary`, …);
                    // skip its value when the next named child arrives.
                    pending_method = Some(String::new());
                }
            }
            continue;
        }
        // We have a key, now this child is the value.
        let method = pending_method.take().unwrap_or_default();
        if !method.is_empty() {
            let handler_line = locate_first_sym_line(child, src)
                .unwrap_or_else(|| map_node.start_position().row as u32 + 1);
            let handler_symbol_index = symbols
                .iter()
                .position(|s| s.start_line == handler_line)
                .unwrap_or(0);
            routes.push(ExtractedRoute {
                handler_symbol_index,
                http_method: method,
                template: template.to_string(),
            });
        }
    }
}

fn reitit_verb_to_method(kw: &str) -> Option<&'static str> {
    match kw {
        "get" => Some("GET"),
        "post" => Some("POST"),
        "put" => Some("PUT"),
        "patch" => Some("PATCH"),
        "delete" => Some("DELETE"),
        "head" => Some("HEAD"),
        "options" => Some("OPTIONS"),
        "any" => Some(""),
        _ => None,
    }
}

fn first_string_child(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        if child.kind() == "str_lit" {
            let text = node_text_str(child, src);
            let trimmed = text.trim_matches('"');
            if trimmed.starts_with('/') {
                return Some(trimmed.to_string());
            }
            return None;
        }
        // First non-string named child means this isn't a route-shaped vec.
        return None;
    }
    None
}

fn combine_reitit_paths(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        return segment.to_string();
    }
    if segment == "/" {
        return parent.to_string();
    }
    let parent_trimmed = parent.trim_end_matches('/');
    let seg_with_slash = if segment.starts_with('/') {
        segment.to_string()
    } else {
        format!("/{}", segment)
    };
    format!("{}{}", parent_trimmed, seg_with_slash)
}

/// Locate the first `sym_lit` line inside a value subtree (Reitit handler
/// references are bare symbols or `{:handler the-handler}` maps).
fn locate_first_sym_line(node: Node, src: &[u8]) -> Option<u32> {
    if node.kind() == "sym_lit" {
        let name = sym_lit_name(node, src);
        if !name.is_empty() {
            return Some(node.start_position().row as u32 + 1);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(line) = locate_first_sym_line(child, src) {
            return Some(line);
        }
    }
    None
}

fn node_text_str(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}
