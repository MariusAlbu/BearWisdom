use super::*;

#[test]
fn profile_place_capture_attests_operator_and_selector_without_decoding_identity() {
    let source = "fn f(p:P) { let a=*p; let b=-p; let c=!p; let d=p.r#type; let e=p.1; }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let forms = crate::languages::rust_lang::namespaces::FORMS.places;
    fn walk(node: Node, source: &str, forms: &PlaceSyntax, seen: &mut Vec<String>) {
        if forms.recognizes(node) {
            let text = node.utf8_text(source.as_bytes()).unwrap();
            match forms.capture(node) {
                Some(Place::Dereference(operand)) => {
                    assert_eq!(text, "*p");
                    assert_eq!(operand.utf8_text(source.as_bytes()).unwrap(), "p");
                }
                Some(Place::Field(_, selector)) => assert!(matches!(
                    selector.utf8_text(source.as_bytes()).unwrap(),
                    "r#type" | "1"
                )),
                None => assert!(matches!(text, "-p" | "!p")),
            }
            seen.push(text.to_owned());
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, source, forms, seen);
        }
    }
    let mut seen = Vec::new();
    walk(tree.root_node(), source, forms, &mut seen);
    assert_eq!(seen.len(), 5);
}
