// =============================================================================
// languages/robot/dynamic_keywords.rs — Robot dynamic-library keyword scan
//
// Robot Framework's "dynamic library" API and HYBRID library style let a
// Python class expose a set of keywords whose names are NOT regular Python
// method identifiers. The four shapes covered:
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
//   3. `@keyword("Custom Name")` (and bare `@keyword`) decorator aliases:
//
//        @keyword("Add ${count} copies of ${item} to cart")
//        def add_copies_to_cart(self, count, item): ...
//
//      The decorator argument is the keyword Robot looks up; the method's
//      Python name is the resolution target.
//
//   4. `get_keyword_names` returning a `dir(self)` comprehension filtered by
//      `name.startswith("PREFIX_")` — registers every method in the class
//      whose name begins with PREFIX_ as a keyword.
//
// All four expose names that the regular Python extractor cannot match by
// `def name():` alone. Without this scan, every Robot call to those keywords
// leaves an unresolved Calls ref.
//
// Out of scope:
//
//   * Names assembled from string concatenation, `.format(...)`, or other
//     dynamic constructions — these aren't statically resolvable.
//   * `dir(self)` filtered by `hasattr(getattr(self, name), "robot_name")` —
//     this is what `@keyword` already sets, so coverage from shape 3 above
//     subsumes it for libraries that decorate consistently.
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
    /// `KEYWORDS = {...}` dict. The resolver uses this to find the
    /// owning class symbol when no specific method target is known.
    pub class_name: Option<String>,
    /// Python method name this keyword maps to, when known — set for
    /// `@keyword("alias")` decorators and for `dir(self)` prefix
    /// expansions where each matching method becomes its own dynamic
    /// keyword. `None` for KEYWORDS-dict / get_keyword_names list
    /// entries that route through `run_keyword` at runtime instead of
    /// pointing at a specific def.
    pub method_name: Option<String>,
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
                    .then(a.method_name.cmp(&b.method_name))
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
                scan_get_keyword_names_fn(&child, src, None, &[], out);
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
    // Pre-pass: collect every method name in this class so the
    // `dir(self).startswith(PREFIX_)` expansion in `get_keyword_names`
    // can iterate them. Includes both bare and decorated definitions.
    let method_names = collect_class_method_names(&body, src);

    // Hybrid-library shape: classes that define `get_keyword_names` and
    // expose keywords through a side dict like
    // `keywords = {"Args Should Have Been": [...], ...}`. The dict's
    // attribute name varies (`keywords`, `KEYWORDS`, `_keywords`,
    // `_kw_table`, ...), so when the class has `get_keyword_names`, treat
    // any class-level string-keyed dict literal as a keyword source. The
    // false-positive risk is contained to dynamic-library classes —
    // ordinary Python classes don't hit this branch.
    let has_get_keyword_names = method_names.iter().any(|m| m == "get_keyword_names");

    let mut cursor = body.walk();
    for stmt in body.children(&mut cursor) {
        match stmt.kind() {
            "expression_statement" => {
                if has_get_keyword_names {
                    scan_class_dict_assignment(&stmt, src, class_name, out);
                } else {
                    scan_top_level_assignment(&stmt, src, class_name, out);
                }
            }
            "function_definition" => {
                scan_get_keyword_names_fn(&stmt, src, class_name, &method_names, out);
            }
            "decorated_definition" => {
                scan_decorated_method(&stmt, src, class_name, out);
                // The decorated function may itself be `get_keyword_names`
                // (rare but legal — e.g., `@staticmethod def
                // get_keyword_names(...)`). Walk the inner def too.
                if let Some(inner) = first_child_of_kind(&stmt, "function_definition") {
                    scan_get_keyword_names_fn(&inner, src, class_name, &method_names, out);
                }
            }
            _ => {}
        }
    }
}

/// Permissive variant of `scan_top_level_assignment` for use inside a
/// class that defines `get_keyword_names`. Accepts any identifier on
/// the left side as long as the right is a string-keyed dict literal.
fn scan_class_dict_assignment(
    stmt: &Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let Some(asn) = first_child_of_kind(stmt, "assignment") else { return };
    let Some(left) = asn.child_by_field_name("left") else { return };
    if left.kind() != "identifier" {
        return;
    }
    let Some(right) = asn.child_by_field_name("right") else { return };
    if right.kind() != "dictionary" {
        return;
    }
    collect_dict_string_keys(&right, src, class_name, out);
}

fn collect_class_method_names(body: &Node, src: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = body.walk();
    for stmt in body.children(&mut cursor) {
        let fn_node = match stmt.kind() {
            "function_definition" => Some(stmt),
            "decorated_definition" => first_child_of_kind(&stmt, "function_definition"),
            _ => None,
        };
        if let Some(f) = fn_node {
            if let Some(name) = f.child_by_field_name("name") {
                if let Ok(s) = name.utf8_text(src) {
                    names.push(s.to_string());
                }
            }
        }
    }
    names
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

/// Recognise `def get_keyword_names(self):` and emit a keyword for every
/// statically-knowable name the method returns. Covers two return shapes:
///
///   * `return [<string literal>, ...]` — list of explicit names
///   * `return [name for name in dir(self) if name.startswith("PREFIX_")]`
///     — every method in the class whose name begins with `PREFIX_`
///
/// The second form is given the class's pre-collected `method_names` so
/// the prefix expansion stays a pure function of the class shape.
fn scan_get_keyword_names_fn(
    fn_node: &Node,
    src: &[u8],
    class_name: Option<&str>,
    method_names: &[String],
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
        if stmt.kind() != "return_statement" {
            continue;
        }
        let mut sc = stmt.walk();
        for child in stmt.children(&mut sc) {
            match child.kind() {
                "list" => {
                    collect_list_strings(&child, src, class_name, out);
                    return;
                }
                "list_comprehension" => {
                    if let Some(prefix) = match_dir_prefix_comprehension(&child, src) {
                        emit_prefix_methods(&prefix, class_name, method_names, out);
                    }
                    return;
                }
                _ => {}
            }
        }
    }
}

/// Examine a method's decorators for `@keyword(...)` (or bare `@keyword`).
/// Both register the method as a Robot keyword: parameterised form supplies
/// an alias name, bare form means the method's own name is the keyword.
/// Either way the resolution target is the Python method itself.
fn scan_decorated_method(
    decorated: &Node,
    src: &[u8],
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    let Some(fn_node) = first_child_of_kind(decorated, "function_definition") else { return };
    let Some(method_name_node) = fn_node.child_by_field_name("name") else { return };
    let Ok(method_name) = method_name_node.utf8_text(src) else { return };

    let mut cursor = decorated.walk();
    for child in decorated.children(&mut cursor) {
        if child.kind() != "decorator" {
            continue;
        }
        // The decorator's expression is the named "@..." — fetched via
        // the only non-`@` child.
        let mut dc = child.walk();
        let expr = child
            .children(&mut dc)
            .find(|n| n.kind() != "@")
            .map(|n| n);
        let Some(expr) = expr else { continue };
        match expr.kind() {
            "identifier" => {
                if expr.utf8_text(src).ok() == Some("keyword") {
                    push_decorator_alias(method_name, method_name, class_name, out);
                }
            }
            "call" => {
                let Some(func) = expr.child_by_field_name("function") else { continue };
                if func.utf8_text(src).ok() != Some("keyword") {
                    continue;
                }
                let Some(args) = expr.child_by_field_name("arguments") else { continue };
                let alias = first_string_arg(&args, src);
                let alias_str = alias.as_deref().unwrap_or(method_name);
                push_decorator_alias(alias_str, method_name, class_name, out);
            }
            _ => {}
        }
    }
}

fn push_decorator_alias(
    keyword_name: &str,
    method_name: &str,
    class_name: Option<&str>,
    out: &mut Vec<RobotDynamicKeyword>,
) {
    out.push(RobotDynamicKeyword {
        normalized_name: normalize_robot_name(keyword_name),
        class_name: class_name.map(|s| s.to_string()),
        method_name: Some(method_name.to_string()),
    });
}

fn first_string_arg(args: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        if let Some(s) = string_literal_value(&arg, src) {
            return Some(s);
        }
    }
    None
}

/// Recognise `[name for name in dir(self) if name.startswith("PREFIX_")]`.
/// Returns the prefix string when the comprehension matches, otherwise
/// `None`. Only the simplest shape is supported — anything fancier
/// (multi-condition `if`, attr access on `dir`, `dir(other_obj)`) bails.
fn match_dir_prefix_comprehension(expr: &Node, src: &[u8]) -> Option<String> {
    if expr.kind() != "list_comprehension" {
        return None;
    }
    let mut has_dir_self = false;
    let mut prefix: Option<String> = None;
    let mut cursor = expr.walk();
    for child in expr.children(&mut cursor) {
        match child.kind() {
            "for_in_clause" => {
                let mut sc = child.walk();
                for c in child.children(&mut sc) {
                    if c.kind() == "call" {
                        let Some(func) = c.child_by_field_name("function") else { continue };
                        if func.utf8_text(src).ok() == Some("dir") {
                            has_dir_self = true;
                        }
                    }
                }
            }
            "if_clause" => {
                if let Some(p) = extract_startswith_prefix(&child, src) {
                    prefix = Some(p);
                }
            }
            _ => {}
        }
    }
    if has_dir_self { prefix } else { None }
}

/// In an `if_clause` body, find a `<name>.startswith(<string>)` call and
/// return the string. Bails on anything more complex.
fn extract_startswith_prefix(if_clause: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = if_clause.walk();
    for child in if_clause.children(&mut cursor) {
        if child.kind() != "call" {
            continue;
        }
        let Some(func) = child.child_by_field_name("function") else { continue };
        if func.kind() != "attribute" {
            continue;
        }
        let Some(attr) = func.child_by_field_name("attribute") else { continue };
        if attr.utf8_text(src).ok() != Some("startswith") {
            continue;
        }
        let Some(args) = child.child_by_field_name("arguments") else { continue };
        return first_string_arg(&args, src);
    }
    None
}

fn emit_prefix_methods(
    prefix: &str,
    class_name: Option<&str>,
    method_names: &[String],
    out: &mut Vec<RobotDynamicKeyword>,
) {
    for method in method_names {
        if !method.starts_with(prefix) {
            continue;
        }
        out.push(RobotDynamicKeyword {
            normalized_name: normalize_robot_name(method),
            class_name: class_name.map(|s| s.to_string()),
            method_name: Some(method.clone()),
        });
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
                method_name: None,
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
                method_name: None,
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
