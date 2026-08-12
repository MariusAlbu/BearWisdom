use super::*;
use tree_sitter::Parser;

/// Parse `source` and return the first descendant node of `kind`, depth-first.
fn find_node(source: &str, kind: &str) -> tree_sitter::Tree {
    let language = tree_sitter_go::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("failed to set Go grammar");
    let tree = parser.parse(source, None).expect("failed to parse");
    assert!(
        find_kind(tree.root_node(), kind).is_some(),
        "no `{kind}` node found in:\n{source}"
    );
    tree
}

fn find_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

fn node_of_kind<'a>(tree: &'a tree_sitter::Tree, kind: &str) -> Node<'a> {
    find_kind(tree.root_node(), kind).unwrap()
}

#[test]
fn qualified_type_parts_splits_package_and_name() {
    let src = "package p\nfunc f(t *testing.T) {}";
    let tree = find_node(src, "qualified_type");
    let node = node_of_kind(&tree, "qualified_type");
    let (package, name) = qualified_type_parts(&node, src).expect("expected parts");
    assert_eq!(package, "testing");
    assert_eq!(name, "T");
}

#[test]
fn go_type_ref_target_bare_identifier_has_no_module() {
    let src = "package p\nfunc f(u User) {}";
    let tree = find_node(src, "type_identifier");
    // The parameter's type_identifier, not the function name — walk to the
    // parameter_list first so we don't grab `f`.
    let root = tree.root_node();
    let params = find_kind(root, "parameter_list").unwrap();
    let node = find_kind(params, "type_identifier").unwrap();
    let (name, module) = go_type_ref_target(&node, src).expect("expected target");
    assert_eq!(name, "User");
    assert_eq!(module, None);
}

#[test]
fn go_type_ref_target_pointer_to_qualified_type_carries_module() {
    // `*testing.T` — pointer_type wrapping qualified_type.
    let src = "package p\nfunc f(t *testing.T) {}";
    let tree = find_node(src, "pointer_type");
    let node = node_of_kind(&tree, "pointer_type");
    let (name, module) = go_type_ref_target(&node, src).expect("expected target");
    assert_eq!(name, "T");
    assert_eq!(module.as_deref(), Some("testing"));
}

#[test]
fn go_type_ref_target_value_qualified_type_carries_module() {
    // `fiber.Config` — no pointer, qualified_type directly.
    let src = "package p\nfunc f(c fiber.Config) {}";
    let tree = find_node(src, "qualified_type");
    let node = node_of_kind(&tree, "qualified_type");
    let (name, module) = go_type_ref_target(&node, src).expect("expected target");
    assert_eq!(name, "Config");
    assert_eq!(module.as_deref(), Some("fiber"));
}

#[test]
fn go_type_ref_target_slice_of_pointer_to_qualified_type() {
    // `[]*foo.Bar` — slice_type wrapping pointer_type wrapping qualified_type.
    let src = "package p\nfunc f(xs []*foo.Bar) {}";
    let tree = find_node(src, "slice_type");
    let node = node_of_kind(&tree, "slice_type");
    let (name, module) = go_type_ref_target(&node, src).expect("expected target");
    assert_eq!(name, "Bar");
    assert_eq!(module.as_deref(), Some("foo"));
}

#[test]
fn go_type_ref_target_generic_base_with_qualified_type_arg() {
    // `Map[string]*pkg.Type` — generic_type base "Map" is bare; the type
    // argument `*pkg.Type` is a separate qualified_type reachable via
    // `type_arguments`, not through this function's base-unwrap path.
    let src = "package p\nfunc f(m Map[string, *pkg.Type]) {}";
    let tree = find_node(src, "generic_type");
    let node = node_of_kind(&tree, "generic_type");
    let (name, module) = go_type_ref_target(&node, src).expect("expected target");
    assert_eq!(name, "Map");
    assert_eq!(module, None);

    // The qualified type argument resolves independently to the same shape
    // every other qualified_type site produces.
    let arg = find_kind(node, "qualified_type").expect("expected qualified type argument");
    let (arg_name, arg_module) = go_type_ref_target(&arg, src).expect("expected target");
    assert_eq!(arg_name, "Type");
    assert_eq!(arg_module.as_deref(), Some("pkg"));
}

#[test]
fn go_type_ref_target_qualified_generic_base() {
    // `pkg.List[int]` — generic_type whose base itself is a qualified_type.
    let src = "package p\nfunc f(l pkg.List[int]) {}";
    let tree = find_node(src, "generic_type");
    let node = node_of_kind(&tree, "generic_type");
    let (name, module) = go_type_ref_target(&node, src).expect("expected target");
    assert_eq!(name, "List");
    assert_eq!(module.as_deref(), Some("pkg"));
}
