use super::*;

#[test]
fn spans_are_utf8_declaration_ranges_and_patterns_keep_positions() {
    let source = "f(({ x }, café: Alpha, ...rest) => café.save())";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let call = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap();
    let lambda = call
        .child_by_field_name("arguments")
        .unwrap()
        .named_child(0)
        .unwrap();
    let spans = parameters(&lambda);
    let start = source.find("café").unwrap() as u32;
    assert_eq!(
        spans,
        vec![
            None,
            Some(SourceSpan {
                start,
                end: start + 5
            }),
            None
        ]
    );
}
