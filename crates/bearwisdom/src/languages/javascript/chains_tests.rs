use super::*;

#[test]
fn source_receiver_kind_is_preserved_through_nested_calls() {
    let source = "class Child extends Parent { run() { super.save().touch(); this.save(); } }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut pending = vec![tree.root_node()];
    let mut kinds = Vec::new();
    while let Some(node) = pending.pop() {
        if node.kind() == "call_expression" {
            if let Some(chain) = build_member_chain(
                node.child_by_field_name("function").unwrap(),
                source.as_bytes(),
            ) {
                kinds.push(chain.segments[0].kind);
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    assert_eq!(
        kinds.iter().filter(|&&k| k == SegmentKind::BaseRef).count(),
        2
    );
    assert_eq!(
        kinds.iter().filter(|&&k| k == SegmentKind::SelfRef).count(),
        1
    );
}
