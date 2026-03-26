use super::*;

/// Minimal scope config for C#-style tests.
const CSHARP_CONFIG: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_declaration", name_field: "name" },
    ScopeKind { node_kind: "class_declaration",     name_field: "name" },
    ScopeKind { node_kind: "method_declaration",    name_field: "name" },
];

fn parse_csharp(source: &str) -> tree_sitter::Tree {
    let lang: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    parser.parse(source, None).unwrap()
}

#[test]
fn build_scopes_for_namespace_class_method() {
    let source = "namespace Foo { class Bar { void Baz() {} } }";
    let tree = parse_csharp(source);
    let scopes = build(tree.root_node(), source.as_bytes(), CSHARP_CONFIG);

    let names: Vec<&str> = scopes.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Foo"),  "Missing Foo:  {names:?}");
    assert!(names.contains(&"Bar"),  "Missing Bar:  {names:?}");
    assert!(names.contains(&"Baz"),  "Missing Baz:  {names:?}");
}

#[test]
fn qualified_names_are_dotted() {
    let source = "namespace A { class B { void C() {} } }";
    let tree = parse_csharp(source);
    let scopes = build(tree.root_node(), source.as_bytes(), CSHARP_CONFIG);

    let c = scopes.iter().find(|s| s.name == "C").unwrap();
    assert_eq!(c.qualified_name, "A.B.C");
}

#[test]
fn find_scope_at_returns_deepest_scope() {
    let source = "namespace A { class B { void C() {} } }";
    let tree = parse_csharp(source);
    let scopes = build(tree.root_node(), source.as_bytes(), CSHARP_CONFIG);

    // Pick an offset inside the method body.
    // `void C()` starts somewhere after the opening brace of B.
    // `{` of C() body is what we're after — just pick end of the string.
    let inside_c_offset = source.find("void C").unwrap() + 5;
    let scope = find_scope_at(&scopes, inside_c_offset).unwrap();
    assert_eq!(scope.name, "C", "Expected deepest scope 'C', got '{}'", scope.name);
}

#[test]
fn qualify_helper_builds_full_name() {
    let entry = ScopeEntry {
        name: "Bar".to_string(),
        qualified_name: "Foo.Bar".to_string(),
        node_kind: "class_declaration",
        start_byte: 0,
        end_byte: 100,
        depth: 1,
    };
    let qname = qualify("GetById", Some(&entry));
    assert_eq!(qname, "Foo.Bar.GetById");
}

#[test]
fn qualify_with_no_scope_returns_bare_name() {
    let qname = qualify("GlobalFunc", None);
    assert_eq!(qname, "GlobalFunc");
}
