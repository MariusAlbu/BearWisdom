use super::*;

#[test]
fn generic_callees_keep_identifiers_nested_hops_and_argument_payloads() {
    for (expression, expected, at) in [
        ("make::<Doc>()", vec!["make"], 0),
        ("api::make::<Doc>()", vec!["api", "make"], 1),
        ("make::<Doc>().get().save()", vec!["make", "get", "save"], 0),
        ("p.make::<Doc>().save()", vec!["p", "make", "save"], 1),
    ] {
        let source = format!("fn f() {{ {expression}; }}");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let function = tree.root_node().named_child(0).unwrap();
        let statement = function
            .child_by_field_name("body")
            .unwrap()
            .named_child(0)
            .unwrap();
        let call = statement.named_child(0).unwrap();
        let chain = build_chain(call.child_by_field_name("function").unwrap(), &source).unwrap();
        assert_eq!(
            chain
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(chain.segments[at].type_args, ["Doc"]);
        if expression.ends_with(".save()") {
            assert!(chain.segments[at].is_call);
        }
    }
}
