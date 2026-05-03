// =============================================================================
// languages/robot/dynamic_keywords.rs — Robot dynamic-library keyword scan
//
// Robot Framework's "dynamic library" API lets a Python class expose a set of
// keywords whose names are NOT regular Python method identifiers. The two
// shapes that account for the bulk of fixtures in the wild are:
//
//   1. A module-level dict literal whose keys are the keyword names:
//
//        KEYWORDS = {
//            "One Arg":    ["arg"],
//            "Two Args":   ["first", "second"],
//        }
//
//      The `class DynamicWithoutKwargs` then uses `self.keywords.keys()` from
//      `get_keyword_names`. We only need to capture the dict keys.
//
//   2. A `get_keyword_names` method that returns a list literal of strings:
//
//        async def get_keyword_names(self):
//            return ["async_keyword"]
//
// Both forms expose names that the regular Python extractor cannot see — they
// are not `def name(...):` declarations. Without this scan, every Robot call
// to those keywords leaves an unresolved Calls ref.
//
// Out of scope (intentionally — best handled in follow-up work):
//
//   * `get_keyword_names` returning `dir(self)` filtered by prefix or by
//     `hasattr(...)` (relies on tracing instance attributes)
//   * `@keyword("Custom Name")` decorator-driven aliases (handled where the
//     Python extractor reads decorators)
//   * Names assembled from string concatenation or `.format(...)` calls
// =============================================================================

use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use super::predicates::normalize_robot_name;

/// One dynamic-keyword binding the Robot resolver can use to redirect a
/// `Calls` ref into a Python library file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotDynamicKeyword {
    /// Robot-normalised lookup form (`Get Keyword That Passes` →
    /// `get_keyword_that_passes`). The resolver builds the same form for
    /// the call site and compares directly.
    pub normalized_name: String,
    /// Class that owns the keyword, or `None` for a module-level
    /// `KEYWORDS = {...}` dict. Used by the resolver to resolve to the
    /// owning Class symbol so the edge points somewhere meaningful even
    /// though there's no per-keyword Python definition.
    pub class_name: Option<String>,
}

/// `python_file_path → keywords_in_that_file`. Empty for files we couldn't
/// parse or that don't follow either shape.
pub type RobotDynamicKeywordMap = HashMap<String, Vec<RobotDynamicKeyword>>;

/// Build the dynamic-keyword map for a set of `.py` files known to be
/// imported as Robot libraries.
///
/// `library_paths` is the deduplicated list of `.py` files reached by the
/// `library_map` pre-pass. The reader closure abstracts over the file source
/// so tests can pass an in-memory map; production wires it to
/// `std::fs::read_to_string`.
pub fn build_robot_dynamic_keyword_map(
    library_paths: &[&str],
    reader: impl Fn(&str) -> Option<String>,
) -> RobotDynamicKeywordMap {
    let mut map = RobotDynamicKeywordMap::new();
    if library_paths.is_empty() {
        return map;
    }

    let language = tree_sitter_python::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return map;
    }

    for path in library_paths {
        let Some(source) = reader(path) else { continue };
        let Some(tree) = parser.parse(&source, None) else { continue };
        let mut keywords: Vec<RobotDynamicKeyword> = Vec::new();
        scan_module(tree.root_node(), source.as_bytes(), &mut keywords);
        if !keywords.is_empty() {
            // Deduplicate — the same keyword can be listed in both a
            // KEYWORDS dict AND a `get_keyword_names` literal in pathological
            // hand-rolled libraries. The resolver only needs one entry.
            keywords.sort_by(|a, b| {
                a.normalized_name
                    .cmp(&b.normalized_name)
                    .then(a.class_name.cmp(&b.class_name))
            });
            keywords.dedup();
            map.insert((*path).to_string(), keywords);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Tree-sitter walk
// ---------------------------------------------------------------------------

fn scan_module(node: Node, src: &[u8], out: &mut Vec<RobotDynamicKeyword>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "expression_statement" => {
                scan_top_level_assignment(&child, src, None, out);
            }
            "class_definition" => {
                let class_name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    .map(|s| s.to_string());
                if let Some(body) = child.child_by_field_name("body") {
                    scan_class_body(body, src, class_name.as_deref(), out);
                }
            }
            "function_definition" => {
                // Module-level `def get_keyword_names()` is unusual but legal.
                scan_get_keyword_names_fn(&child, src, None, out);
            }
            _ => {}
        }
    }
}

fn scan_class_body(
    body: Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let mut cursor = body.walk();
    for stmt in body.children(&mut cursor) {
        match stmt.kind() {
            "expression_statement" => {
                scan_top_level_assignment(&stmt, src, class_name, out);
            }
            "function_definition" => {
                scan_get_keyword_names_fn(&stmt, src, class_name, out);
            }
            _ => {}
        }
    }
}

/// Recognise an `expression_statement` wrapping `KEYWORDS = {dict literal}`
/// and emit a keyword for each string key.
fn scan_top_level_assignment(
    stmt: &Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let Some(asn) = first_child_of_kind(stmt, "assignment") else { return };
    let Some(left) = asn.child_by_field_name("left") else { return };
    let Ok(left_text) = left.utf8_text(src) else { return };
    if left_text != "KEYWORDS" {
        return;
    }
    let Some(right) = asn.child_by_field_name("right") else { return };
    if right.kind() != "dictionary" {
        return;
    }
    collect_dict_string_keys(&right, src, class_name, out);
}

/// Recognise `def get_keyword_names(self):` and emit a keyword for each
/// string in the first `return [list literal]` we find in the body.
fn scan_get_keyword_names_fn(
    fn_node: &Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let Some(name_node) = fn_node.child_by_field_name("name") else { return };
    let Ok(name) = name_node.utf8_text(src) else { return };
    if name != "get_keyword_names" {
        return;
    }
    let Some(body) = fn_node.child_by_field_name("body") else { return };
    let mut cursor = body.walk();
    for stmt in body.children(&mut cursor) {
        if stmt.kind() == "return_statement" {
            // The return expression is the first non-keyword child.
            let mut sc = stmt.walk();
            for child in stmt.children(&mut sc) {
                if child.kind() == "list" {
                    collect_list_strings(&child, src, class_name, out);
                    return;
                }
            }
        }
    }
}

fn collect_dict_string_keys(
    dict: &Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let mut cursor = dict.walk();
    for child in dict.children(&mut cursor) {
        if child.kind() != "pair" {
            continue;
        }
        let Some(key) = child.child_by_field_name("key") else { continue };
        if let Some(s) = string_literal_value(&key, src) {
            out.push(RobotDynamicKeyword {
                normalized_name: normalize_robot_name(&s),
                class_name: class_name.map(|s| s.to_string()),
            });
        }
    }
}

fn collect_list_strings(
    list: &Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if let Some(s) = string_literal_value(&child, src) {
            out.push(RobotDynamicKeyword {
                normalized_name: normalize_robot_name(&s),
                class_name: class_name.map(|s| s.to_string()),
            });
        }
    }
}

/// Return the text content of a Python string literal, stripped of its
/// quote markers. Handles single, double, triple, and prefixed (`r"..."`,
/// `b"..."`, `f"..."`) forms via the same approach the Python extractor
/// uses for decorator string args. f-string interpolations would still
/// flow through as raw text but those are rare in dynamic keyword names.
fn string_literal_value(node: &Node, src: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let raw = node.utf8_text(src).ok()?;
    let stripped = raw
        .trim_start_matches(|c: char| c == 'r' || c == 'R' || c == 'b' || c == 'B' || c == 'u' || c == 'U' || c == 'f' || c == 'F')
        .trim_start_matches("\"\"\"")
        .trim_end_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("'''")
        .trim_matches('"')
        .trim_matches('\'');
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.to_string())
}

fn first_child_of_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
#[path = "dynamic_keywords_tests.rs"]
mod tests;
