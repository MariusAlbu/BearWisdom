use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Decorator extraction
//
// Tree-sitter represents decorators differently depending on where they appear:
//
// Class decorators are direct children of the `class_declaration` node:
//   class_declaration
//     decorator          ← child, before `class` keyword
//     class "class"
//     type_identifier "UserController"
//     class_body { ... }
//
// Method decorators are preceding siblings of the `method_definition` node
// inside a `class_body`:
//   class_body
//     { "{"
//     decorator          ← sibling before method_definition
//     method_definition
//       property_identifier "findOne"
//
// Decorator forms:
//   @Injectable()              → call_expression → identifier "Injectable"
//   @Controller('/api/users')  → call_expression → identifier + string arg
//   @Roles.Admin()             → call_expression → member_expression
//   @Component({ ... })        → call_expression → identifier + object arg
// ---------------------------------------------------------------------------

/// Extract decorator metadata from a class or method node.
///
/// Emits one `ExtractedRef` with `kind: EdgeKind::TypeRef` per decorator.
/// The `target_name` is the decorator name (e.g. `Injectable`, `Controller`).
/// The `module` field stores the first string argument when present
/// (route path, event name, etc.).
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let decorator_nodes = collect_decorator_nodes(node);
    for dec in decorator_nodes {
        if let Some((name, first_arg)) = parse_decorator(&dec, src) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::TypeRef,
                line: dec.start_position().row as u32,
                module: first_arg,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
    }
}

// ---------------------------------------------------------------------------
// Collect the decorator nodes relevant to a given class or method node.
// ---------------------------------------------------------------------------

fn collect_decorator_nodes<'a>(node: &'a Node<'a>) -> Vec<Node<'a>> {
    match node.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            // Decorators may be direct children of the class node itself
            // (non-exported class), OR children of the wrapping
            // `export_statement` when `export class Foo {}` is used. In the
            // latter case the AST looks like:
            //   export_statement
            //     decorator   ← sibling of class_declaration, child of export_statement
            //     export
            //     class_declaration
            // Check the parent node first; fall back to direct children.
            let search_node = if let Some(parent) = node.parent() {
                if parent.kind() == "export_statement" { parent } else { *node }
            } else {
                *node
            };
            let mut result = Vec::new();
            let mut cursor = search_node.walk();
            for child in search_node.children(&mut cursor) {
                if child.kind() == "decorator" {
                    result.push(child);
                }
            }
            result
        }
        "method_definition" | "method_signature" => {
            // Decorators are preceding siblings in the class body.
            let mut result = Vec::new();
            let mut sib = node.prev_sibling();
            while let Some(s) = sib {
                if s.kind() == "decorator" {
                    result.push(s);
                } else {
                    break;
                }
                sib = s.prev_sibling();
            }
            // Collected in reverse order — reverse so they read top-to-bottom.
            result.reverse();
            result
        }
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Parse a single `decorator` node into (name, optional_first_arg).
// ---------------------------------------------------------------------------

fn parse_decorator<'a>(node: &'a Node<'a>, src: &[u8]) -> Option<(String, Option<String>)> {
    // A decorator node has a single meaningful child:
    //   call_expression  → @Foo(...)  or @Foo.Bar(...)
    //   identifier       → @Foo  (bare, no parens)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                let name = extract_call_name(&child, src)?;
                let first_arg = extract_first_string_arg(&child, src);
                return Some((name, first_arg));
            }
            "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    return Some((name, None));
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract the decorator name from the `function` field of a `call_expression`.
///
/// Handles:
///   identifier          → "Injectable"
///   member_expression   → "Roles" (we take the object, not the property)
fn extract_call_name(call_expr: &Node, src: &[u8]) -> Option<String> {
    let func = call_expr.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => {
            let name = node_text(func, src);
            if name.is_empty() { None } else { Some(name) }
        }
        "member_expression" => {
            // @Roles.Admin() — use the object name as the primary decorator name.
            let object = func.child_by_field_name("object")?;
            let name = node_text(object, src);
            if name.is_empty() { None } else { Some(name) }
        }
        _ => None,
    }
}

/// Return the first string literal argument from a `call_expression`'s `arguments`.
fn extract_first_string_arg(call_expr: &Node, src: &[u8]) -> Option<String> {
    let args = call_expr.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(child, src);
            // Strip surrounding quotes (single or double).
            let stripped = raw.trim_matches('"').trim_matches('\'').to_string();
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Angular @Component selector extraction
// ---------------------------------------------------------------------------

/// Extract the `selector` field value(s) from an `@Component({...})` decorator
/// on a class node.
///
/// Returns the raw selector strings (not yet normalized) extracted from the
/// `selector: '...'` property of the decorator's object argument. Multiple
/// selectors separated by commas are returned as individual strings. Bracket
/// wrappers for attribute selectors (`[appHighlight]` → `appHighlight`) and
/// dot prefixes for class selectors (`.my-class` → `my-class`) are stripped.
///
/// Returns an empty vec when the class has no `@Component` decorator or the
/// decorator has no parseable `selector` property.
pub(crate) fn component_selectors_from_class(node: &Node, src: &[u8]) -> Vec<String> {
    let decorator_nodes = collect_decorator_nodes(node);
    for dec in &decorator_nodes {
        if let Some(selectors) = try_extract_selector_from_decorator(dec, src) {
            return selectors;
        }
    }
    Vec::new()
}

/// Try to extract the `selector` property from a single `decorator` node.
/// Returns `Some(selectors)` only when the decorator is named `Component`
/// and has a parseable `selector` value.
fn try_extract_selector_from_decorator(dec: &Node, src: &[u8]) -> Option<Vec<String>> {
    let mut cursor = dec.walk();
    for child in dec.children(&mut cursor) {
        if child.kind() != "call_expression" {
            continue;
        }
        // Verify this is `@Component(...)` — not `@Directive(...)` etc.
        let func = child.child_by_field_name("function")?;
        let dec_name = match func.kind() {
            "identifier" => node_text(func, src),
            _ => continue,
        };
        if dec_name != "Component" {
            continue;
        }
        // Walk the arguments list for an object expression.
        let args = child.child_by_field_name("arguments")?;
        let mut ac = args.walk();
        for arg in args.children(&mut ac) {
            if arg.kind() == "object" {
                if let Some(selectors) = extract_selector_from_object(&arg, src) {
                    return Some(selectors);
                }
            }
        }
    }
    None
}

/// Walk an `object` node (the `@Component({...})` argument) and extract the
/// `selector` property's string value(s).
fn extract_selector_from_object(obj: &Node, src: &[u8]) -> Option<Vec<String>> {
    let mut cursor = obj.walk();
    for prop in obj.children(&mut cursor) {
        // Property nodes: `pair`, `shorthand_property_identifier`
        if prop.kind() != "pair" {
            continue;
        }
        let key = prop.child_by_field_name("key")?;
        let key_text = node_text(key, src);
        if key_text != "selector" {
            continue;
        }
        let val = prop.child_by_field_name("value")?;
        let raw_value = unquote_string_node(&val, src)?;
        return Some(split_and_normalize_selectors(&raw_value));
    }
    None
}

/// Strip surrounding quotes from a `string` or `template_string` node.
fn unquote_string_node(node: &Node, src: &[u8]) -> Option<String> {
    let raw = node_text(*node, src);
    if raw.is_empty() {
        return None;
    }
    let stripped = raw
        .trim_start_matches('`')
        .trim_end_matches('`')
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string();
    if stripped.is_empty() { None } else { Some(stripped) }
}

/// Split a raw selector string on commas and normalize each part:
///
///   - Element selectors: `"app-user-card"` → `"app-user-card"` (unchanged)
///   - Attribute selectors: `"[appHighlight]"` → `"appHighlight"`
///   - Class selectors: `".my-class"` → `"my-class"`
///   - Comma lists: `"app-foo, [bar], .baz"` → `["app-foo", "bar", "baz"]`
pub(crate) fn split_and_normalize_selectors(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| normalize_single_selector(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn normalize_single_selector(s: &str) -> String {
    if s.starts_with('[') && s.ends_with(']') {
        // Attribute selector: strip `[` and `]`, then strip any `=...` suffix.
        let inner = &s[1..s.len() - 1];
        // `[ngModel]` → `ngModel`; `[ngModel]="x"` is not a selector value form
        // but handle `[(ngModel)]` as Angular two-way binding: strip `(` and `)`.
        inner
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split('=')
            .next()
            .unwrap_or(inner)
            .to_string()
    } else if s.starts_with('.') {
        // Class selector: strip leading `.`
        s[1..].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "decorators_tests.rs"]
mod tests;
