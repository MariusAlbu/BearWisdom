use super::*;

#[test]
fn capture_exact_bodies_and_reject_mutation_async_and_nested_returns() {
    let source = "api.pick(value => true); api.pick(value => typeof value === 'string'); api.pick(value => value.ready()); api.pick(value => { value = 1; return true; }); api.pick(async value => true); api.pick(value => { return (() => true)(); });";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut calls = super::super::Capture::default();
    super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
        &mut calls,
    );
    let mut values: Vec<_> = calls.callbacks.values().collect();
    values.sort_by_key(|c| c.signature.start);
    assert_eq!(values.len(), 6);
    assert!(matches!(values[0].body, Expr::Atom(_)));
    assert!(matches!(
        values[1].body,
        Expr::TypeTest {
            kind: Intrinsic::String,
            negated: false,
            ..
        }
    ));
    assert!(matches!(values[2].body, Expr::Call { .. }));
    assert!(values[3..].iter().all(|c| matches!(c.body, Expr::Unknown)));
    assert_eq!(
        values[0].parameters[0].start,
        source.find("value").unwrap() as u32
    );
}
