// =============================================================================
// python/decorators.rs  —  Decorator extraction for Python
//
// Python decorator forms:
//   @app.route('/api/users')     → attribute (dotted) + call with string arg
//   @dataclass                   → bare identifier
//   @pytest.mark.parametrize(…)  → attribute (dotted) + call with arg
//
// Tree-sitter shape inside a `decorated_definition`:
//   decorator
//     identifier          → @dataclass
//   decorator
//     call
//       function: attribute | identifier
//       arguments: argument_list
//   decorator
//     attribute           → @pytest.mark.parametrize (rare, no parens)
//
// NOTE: the python/symbols.rs already uses `extract_decorator_names` to figure
// out `@property` / `@pytest.mark…` for SymbolKind decisions.  This module
// emits EdgeKind::TypeRef refs so those decorator names appear in the graph.
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Emit one `ExtractedRef` per decorator attached to `decorated_def_node`.
///
/// `decorated_def_node` must be a `decorated_definition` node.
/// `source_symbol_index` is the index of the symbol that was just pushed for
/// the class or function that this decorated_definition wraps.
pub(super) fn extract_decorators(
    decorated_def_node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = decorated_def_node.walk();
    for child in decorated_def_node.children(&mut cursor) {
        if child.kind() == "decorator" {
            if let Some((name, first_arg)) = parse_decorator(&child, source) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: first_arg,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parse a single `decorator` node
// ---------------------------------------------------------------------------

fn parse_decorator(node: &Node, source: &str) -> Option<(String, Option<String>)> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // @dataclass
            "identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    return Some((name, None));
                }
            }
            // @pytest.mark.parametrize  (bare, no parens)
            "attribute" => {
                let name = attribute_name(&child, source);
                if !name.is_empty() {
                    return Some((name, None));
                }
            }
            // @app.route('/api/users')  or  @dataclass()
            "call" => {
                let func = child.child_by_field_name("function")?;
                let name = match func.kind() {
                    "identifier" => node_text(&func, source),
                    "attribute" => attribute_name(&func, source),
                    _ => continue,
                };
                if name.is_empty() {
                    continue;
                }
                let first_arg = extract_first_string_arg(&child, source);
                return Some((name, first_arg));
            }
            _ => {}
        }
    }
    None
}

/// Extract the dotted name from an `attribute` node.
/// For `app.route` returns `"app.route"`, for `pytest.mark.parametrize` returns
/// `"pytest.mark.parametrize"`.  We use the full text so callers can recognise
/// framework-specific patterns.
fn attribute_name(node: &Node, source: &str) -> String {
    node_text(node, source)
}

/// Return the text of the first string literal in the `arguments` of a `call`.
fn extract_first_string_arg(call_node: &Node, source: &str) -> Option<String> {
    let args = call_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(&child, source);
            // Strip surrounding quotes (single, double, triple).
            let stripped = raw
                .trim_start_matches("\"\"\"")
                .trim_end_matches("\"\"\"")
                .trim_start_matches("'''")
                .trim_end_matches("'''")
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use crate::types::EdgeKind;

    fn decorator_refs(source: &str) -> Vec<(String, Option<String>)> {
        extract(source)
            .refs
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.target_name, r.module))
            .collect()
    }

    #[test]
    fn bare_decorator() {
        let src = "@dataclass\nclass Point:\n    x: int = 0\n";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "dataclass"), "refs: {dr:?}");
    }

    #[test]
    fn decorator_with_route_arg() {
        let src = "@app.route('/api/users')\ndef users():\n    pass\n";
        let dr = decorator_refs(src);
        let found = dr.iter().find(|(n, _)| n == "app.route");
        assert!(found.is_some(), "refs: {dr:?}");
        assert_eq!(found.unwrap().1, Some("/api/users".to_string()));
    }

    #[test]
    fn decorator_no_args_call() {
        let src = "@login_required()\ndef view():\n    pass\n";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "login_required"), "refs: {dr:?}");
    }

    #[test]
    fn multiple_decorators_on_function() {
        let src = "@csrf_exempt\n@login_required\ndef action():\n    pass\n";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "csrf_exempt"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "login_required"), "refs: {dr:?}");
    }

    #[test]
    fn decorator_on_class() {
        let src = "@Injectable\nclass Service:\n    pass\n";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Injectable"), "refs: {dr:?}");
    }
}
