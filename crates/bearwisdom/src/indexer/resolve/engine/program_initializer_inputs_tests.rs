use super::*;

#[test]
fn portable_initializer_retains_signature_origin_without_a_navigation_row() {
    let input = Input {
        declaration: None,
        signature: SignatureId(crate::types::SourceSpan { start: 12, end: 39 }),
        target: Some(crate::types::SourceSpan { start: 12, end: 18 }),
        expression: Expression::Construct {
            callee: crate::types::SourceSpan { start: 24, end: 30 },
            arguments: vec![],
            types: vec![Recipe::Parameter { owner: 9, index: 0 }],
        },
        annotated: false,
    };
    let restored: Input = serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
    assert_eq!(restored.signature, input.signature);
    assert_eq!(restored.declaration, None);
    assert_eq!(restored.target, input.target);
    let Expression::Construct { types, .. } = restored.expression else {
        panic!()
    };
    assert!(matches!(types[0], Recipe::Parameter { owner: 9, index: 0 }));
}
