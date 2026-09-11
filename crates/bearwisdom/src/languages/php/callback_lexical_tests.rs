use super::*;

fn parse(source: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .unwrap();
    parser.parse(source, None).unwrap()
}

fn find_closure<'a>(root: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "anonymous_function" | "arrow_function") {
            return Some(node);
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        stack.extend(children);
    }
    None
}

#[test]
fn closure_parameters_keep_their_type_annotation() {
    let source = b"<?php\nSchema::create('t', function (Blueprint $table, $raw, ?int $n = null) { $table->string('x'); });\n";
    let tree = parse(std::str::from_utf8(source).unwrap());
    let closure = find_closure(tree.root_node()).expect("closure node");
    let descriptor = describe(closure, source).expect("callback descriptor");
    let params: Vec<(String, Option<String>)> = descriptor
        .parameters
        .iter()
        .map(|p| (p.name.clone(), p.annotation.clone()))
        .collect();
    assert_eq!(
        params,
        vec![
            ("table".to_string(), Some("Blueprint".to_string())),
            ("raw".to_string(), None),
            ("n".to_string(), Some("?int".to_string())),
        ]
    );
}

#[test]
fn arrow_function_parameters_are_described_too() {
    let source = b"<?php\n$f = fn (Request $request) => $request->all();\n";
    let tree = parse(std::str::from_utf8(source).unwrap());
    let closure = find_closure(tree.root_node()).expect("arrow node");
    let descriptor = describe(closure, source).expect("callback descriptor");
    assert_eq!(descriptor.parameters.len(), 1);
    assert_eq!(descriptor.parameters[0].name, "request");
    assert_eq!(descriptor.parameters[0].annotation.as_deref(), Some("Request"));
}
