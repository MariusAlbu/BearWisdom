use super::*;

/// Parse `src` as TypeScript, locate the first `type_alias_declaration`, and
/// collect the branches of its `union_type` right-hand side.
fn branches_of(src: &str) -> (Vec<String>, bool) {
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "type_alias_declaration" {
            let value = child
                .child_by_field_name("value")
                .expect("type_alias_declaration has a value field");
            assert_eq!(value.kind(), "union_type", "RHS is not a union");
            return union_branches(&value, src.as_bytes());
        }
    }
    panic!("no type_alias_declaration in source");
}

#[test]
fn two_arm_union_keeps_both_branches() {
    let (branches, has_object) = branches_of("type Keys = 'click' | 'change';");
    assert_eq!(branches, vec!["'click'".to_string(), "'change'".to_string()]);
    assert!(!has_object);
}

#[test]
fn many_arm_union_keeps_every_branch() {
    // Three or more arms nest left, so a flat child scan would keep only the
    // last one and a mapped type over this union would have a single key.
    let (branches, _) = branches_of("type Keys = 'a' | 'b' | 'c' | 'd';");
    assert_eq!(branches, vec!["'a'", "'b'", "'c'", "'d'"]);
}

#[test]
fn leading_pipe_multiline_union_keeps_every_branch() {
    let (branches, _) = branches_of("type Keys =\n  | 'a'\n  | 'b'\n  | 'c';");
    assert_eq!(branches, vec!["'a'", "'b'", "'c'"]);
}

#[test]
fn named_branches_flatten_across_the_spine() {
    let (branches, has_object) = branches_of("type T = Foo | Bar<Baz> | Qux;");
    assert_eq!(branches, vec!["Foo", "Bar<Baz>", "Qux"]);
    assert!(!has_object);
}

#[test]
fn anonymous_object_arms_are_reported_and_name_nothing() {
    let (branches, has_object) = branches_of("type T = {a: A} | {b: B} | {c: C};");
    assert!(branches.is_empty());
    assert!(has_object);
}

#[test]
fn object_arm_nested_in_the_spine_is_still_seen() {
    // The object arm is the FIRST of three, so it sits at the deepest nesting
    // level — only the recursive walk reaches it.
    let (branches, has_object) = branches_of("type T = {a: A} | Foo | Bar;");
    assert_eq!(branches, vec!["Foo", "Bar"]);
    assert!(has_object);
}
