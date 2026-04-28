//! Tests for TypeScript decorator extraction, including the Angular
//! `@Component` selector metadata path added in PR 18.

use crate::languages::typescript::extract::extract;
use crate::languages::typescript::decorators::{component_selectors_from_class, split_and_normalize_selectors};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Parser;

fn refs(source: &str) -> Vec<ExtractedRef> {
    extract(source, false).refs
}

fn decorator_refs(source: &str) -> Vec<ExtractedRef> {
    refs(source)
        .into_iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .collect()
}

// ---------------------------------------------------------------------------
// Migrated inline tests
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Angular @Component selector extraction (PR 18)
// ---------------------------------------------------------------------------

/// Helper: parse the source with tree-sitter TypeScript and call
/// `component_selectors_from_class` on the first class_declaration found.
fn selectors_from_first_class(source: &str) -> Vec<String> {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let src = source.as_bytes();
    let root = tree.root_node();

    // Walk to find first class_declaration
    fn find_class<'a>(node: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == "class_declaration" || node.kind() == "abstract_class_declaration" {
            return Some(node);
        }
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if let Some(n) = find_class(child) {
                return Some(n);
            }
        }
        None
    }

    let Some(class_node) = find_class(root) else { return Vec::new() };
    component_selectors_from_class(&class_node, src)
}

#[test]
fn component_element_selector() {
    let src = r#"
@Component({
    selector: 'app-user-card',
    templateUrl: './user-card.component.html',
})
export class UserCardComponent {}
"#;
    let selectors = selectors_from_first_class(src);
    assert_eq!(selectors, vec!["app-user-card"]);
}

#[test]
fn component_attribute_selector() {
    let src = r#"
@Directive({
    selector: '[appHighlight]',
})
export class HighlightDirective {}
"#;
    // `@Directive` — not `@Component`, so no selector emitted.
    let selectors = selectors_from_first_class(src);
    assert!(selectors.is_empty(), "Directive should not match Component selector extraction");
}

#[test]
fn component_at_component_only() {
    // @Directive should NOT be picked up — only @Component.
    let src = r#"
@Directive({ selector: '[myDir]' })
export class MyDirective {}
"#;
    let selectors = selectors_from_first_class(src);
    assert!(selectors.is_empty());
}

#[test]
fn component_single_quote_selector() {
    let src = r#"
@Component({ selector: 'lib-button' })
export class ButtonComponent {}
"#;
    let selectors = selectors_from_first_class(src);
    assert_eq!(selectors, vec!["lib-button"]);
}

#[test]
fn component_no_decorator() {
    let src = "export class PlainClass {}";
    let selectors = selectors_from_first_class(src);
    assert!(selectors.is_empty());
}

// ---------------------------------------------------------------------------
// split_and_normalize_selectors unit tests
// ---------------------------------------------------------------------------

#[test]
fn normalize_element_selector_unchanged() {
    let result = split_and_normalize_selectors("app-user-card");
    assert_eq!(result, vec!["app-user-card"]);
}

#[test]
fn normalize_attribute_selector_strips_brackets() {
    let result = split_and_normalize_selectors("[appHighlight]");
    assert_eq!(result, vec!["appHighlight"]);
}

#[test]
fn normalize_class_selector_strips_dot() {
    let result = split_and_normalize_selectors(".my-class");
    assert_eq!(result, vec!["my-class"]);
}

#[test]
fn normalize_comma_list_splits() {
    let result = split_and_normalize_selectors("app-foo, [barDir], .baz");
    assert_eq!(result, vec!["app-foo", "barDir", "baz"]);
}

#[test]
fn normalize_two_way_binding_strips_parens() {
    // `[(ngModel)]` in a selector value (rare but valid in tests)
    let result = split_and_normalize_selectors("[(ngModel)]");
    assert_eq!(result, vec!["ngModel"]);
}
