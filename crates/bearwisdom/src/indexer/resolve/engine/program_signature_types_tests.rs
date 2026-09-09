use super::*;
use crate::types::SourceSpan;

#[test]
fn source_signature_recipe_roundtrip_never_requires_a_navigation_row() {
    let owner = SignatureId(SourceSpan { start: 17, end: 89 });
    let input = Input {
        id: owner,
        declaration: None,
        syntax: Default::default(),
        generics: vec![Generic {
            name: None,
            constraint: None,
            default: None,
        }],
        parameters: vec![Recipe::SignatureParameter { owner, index: 0 }],
        result: Some(Recipe::SignatureParameter { owner, index: 0 }),
    };
    let payload = serde_json::to_value(&input).unwrap();
    let restored: Input = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), payload);
}

#[test]
fn rowless_generic_arenas_are_independent_even_with_identical_spelling_and_spans() {
    let id = SignatureId(SourceSpan { start: 17, end: 89 });
    let input = super::super::Input {
        source_signatures: vec![Input {
            id,
            declaration: None,
            syntax: Default::default(),
            generics: vec![Generic {
                name: None,
                constraint: None,
                default: None,
            }],
            parameters: vec![],
            result: None,
        }],
        ..Default::default()
    };
    let arena = TypeArena::new();
    let a = allocate(&input, &Default::default(), &Default::default(), &arena);
    let b = allocate(&input, &Default::default(), &Default::default(), &arena);
    assert_ne!(a[&id].generic_parameters, b[&id].generic_parameters);
}
