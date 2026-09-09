use super::*;

#[test]
fn source_calls_keep_literal_identity_and_authoritative_nested_call_barriers() {
    let source =
        "const undefined = 42; api.pick('a\\n'); api.pick(42); api.pick(undefined); maker()();";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut result = Capture::default();
    capture(
        tree.root_node(),
        source.as_bytes(),
        &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
        &mut result,
    );
    let site = source.find("'a\\n'").unwrap() as u32;
    assert!(matches!(
        result.atoms[&SourceSpan {
            start: site,
            end: site + 5
        }],
        Atom::Literal(_)
    ));
    assert!(result.arguments[&(source.find("maker()").unwrap() as u32)].is_none());
    let selector = source.find("pick(undefined)").unwrap() as u32;
    assert!(matches!(
        result.arguments[&selector].as_deref().unwrap(),
        [CallArg::IdentAt(_)]
    ));
}
