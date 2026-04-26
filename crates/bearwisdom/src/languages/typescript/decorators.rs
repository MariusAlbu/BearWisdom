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
            // Decorators are direct children of the class node.
            let mut result = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::languages::typescript::extract::extract;
    use crate::types::{EdgeKind, ExtractedRef};

    fn refs(source: &str) -> Vec<ExtractedRef> {
        extract(source, false).refs
    }

    fn decorator_refs(source: &str) -> Vec<ExtractedRef> {
        refs(source)
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .collect()
    }

    #[test]
    fn class_decorator_no_args() {
        let src = "@Injectable()\nclass UserService {}";
        let dr = decorator_refs(src);
        assert!(
            dr.iter().any(|r| r.target_name == "Injectable"),
            "refs: {dr:?}"
        );
    }

    #[test]
    fn class_decorator_with_route_arg() {
        let src = r#"@Controller('/api/users')
class UserController {}"#;
        let dr = decorator_refs(src);
        let ctrl = dr.iter().find(|r| r.target_name == "Controller");
        assert!(ctrl.is_some(), "refs: {dr:?}");
        assert_eq!(ctrl.unwrap().module, Some("/api/users".to_string()));
    }

    #[test]
    fn multiple_class_decorators() {
        let src = "@Injectable()\n@Controller('/users')\nclass C {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|r| r.target_name == "Injectable"), "refs: {dr:?}");
        assert!(dr.iter().any(|r| r.target_name == "Controller"), "refs: {dr:?}");
    }

    #[test]
    fn method_decorator_no_args() {
        let src = "class C {\n    @Get()\n    find() {}\n}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|r| r.target_name == "Get"), "refs: {dr:?}");
    }

    #[test]
    fn method_decorator_with_path() {
        let src = r#"class C {
    @Get(':id')
    findOne() {}
}"#;
        let dr = decorator_refs(src);
        let get = dr.iter().find(|r| r.target_name == "Get");
        assert!(get.is_some(), "refs: {dr:?}");
        assert_eq!(get.unwrap().module, Some(":id".to_string()));
    }

    #[test]
    fn member_expression_decorator() {
        // @Roles.Admin() → decorator name is "Roles"
        let src = "class C {\n    @Roles.Admin()\n    admin() {}\n}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|r| r.target_name == "Roles"), "refs: {dr:?}");
    }

    #[test]
    fn no_decorators_no_extra_refs() {
        let src = "class Svc { find() {} }";
        let dr = decorator_refs(src);
        // Only heritage / type refs from the class itself — no decorator refs.
        assert!(
            dr.iter().all(|r| r.target_name != "Injectable" && r.target_name != "Get"),
            "unexpected refs: {dr:?}"
        );
    }
}
