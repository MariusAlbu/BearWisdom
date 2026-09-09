use super::*;

#[test]
fn infer_binder_has_true_branch_scope_but_does_not_shadow_false_branch() {
    let source = "type Outer<U, T> = T extends { value: infer U } ? U : U;";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(!tree.root_node().has_error());
    let graph = crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let capture = Capture {
        source: source.as_bytes(),
        syntax: &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
        graph: &graph,
        anchors: Default::default(),
    };
    let declaration = tree.root_node().named_child(0).unwrap();
    let TypeExpr::Operator(op) = capture.expr(declaration.child_by_field_name("value").unwrap())
    else {
        panic!("conditional recipe");
    };
    let TypeOperator::Conditional {
        when_true,
        when_false,
        extends,
        ..
    } = *op
    else {
        panic!("conditional");
    };
    let TypeExpr::SignatureParameter {
        owner: inferred, ..
    } = when_true
    else {
        panic!("inferred owner");
    };
    let TypeExpr::SignatureParameter { owner: outer, .. } = when_false else {
        panic!("outer owner");
    };
    assert_ne!(inferred, outer);
    assert_eq!(inferred.0.start, source.find("infer U").unwrap() as u32);
    let TypeExpr::Operator(pattern) = extends else {
        panic!("pattern");
    };
    let TypeOperator::Object(properties) = *pattern else {
        panic!("properties");
    };
    assert!(
        matches!(&properties[0].value, TypeExpr::Operator(op) if matches!(op.as_ref(), TypeOperator::Infer(TypeExpr::SignatureParameter { owner, index: 0 }) if *owner == inferred))
    );
    assert!(graph
        .types
        .signatures
        .iter()
        .any(|s| s.id == inferred && s.declaration.is_none()));
}

#[test]
fn constrained_and_nested_infer_owners_are_source_distinct() {
    let source = "type Choice<T> = T extends { a: infer U extends string } ? U extends { b: infer U } ? U : U : never;";
    for grammar in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error());
        let graph = crate::indexer::lexical::capture(
            tree.root_node(),
            source.as_bytes(),
            "ts",
            &mut vec![],
            &[],
            crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
        )
        .unwrap();
        let expected: Vec<_> = source
            .match_indices("infer U")
            .map(|(byte, _)| byte as u32)
            .collect();
        let signatures: Vec<_> = graph
            .types
            .signatures
            .iter()
            .filter(|s| expected.contains(&s.id.0.start))
            .collect();
        assert_eq!(signatures.len(), 2);
        assert_ne!(signatures[0].id, signatures[1].id);
        assert!(matches!(
            signatures[0].generics[0].constraint,
            Some(TypeExpr::Intrinsic(
                crate::type_checker::core::types::Intrinsic::String
            ))
        ));
        assert!(signatures[1].generics[0].constraint.is_none());
        let name = graph.name_id("U").unwrap();
        let true_byte = source.find("? U : U").unwrap() as u32 + 2;
        let false_byte = true_byte + 4;
        let owner = |byte| {
            *graph
                .type_parameter_sites
                .get(&graph.type_binding_at(byte, name).unwrap())
                .unwrap()
        };
        assert_eq!(owner(true_byte), signatures[1].id);
        assert_eq!(owner(false_byte), signatures[0].id);
    }
}
