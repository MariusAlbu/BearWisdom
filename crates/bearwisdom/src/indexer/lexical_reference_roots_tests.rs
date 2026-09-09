use super::*;

#[test]
fn ingestion_roots_follow_profile_wrappers_and_keep_source_spans() {
    let source = "(api.read()).next();";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let root = reference_root(
        tree.root_node(),
        0,
        &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
    )
    .unwrap();
    assert_eq!((root.start_byte(), root.end_byte()), (1, 4));
}
