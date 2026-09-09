use super::*;

#[test]
fn persisted_type_bases_preserve_import_and_generic_ids() {
    let input = Input {
        inherited_members: Default::default(),
        owner: 41,
        bases: Some(vec![Recipe::Apply(
            Box::new(Recipe::Import(7)),
            vec![Recipe::Parameter {
                owner: 41,
                index: 0,
            }],
        )]),
        plain_parameters: true,
        generic_signature: None,
        surface: Some(vec![]),
    };
    let encoded = serde_json::to_value(input).unwrap();
    let decoded: Input = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    let malformed = Input {
        inherited_members: Default::default(),
        owner: 41,
        bases: None,
        plain_parameters: true,
        generic_signature: None,
        surface: None,
    };
    assert_ne!(
        serde_json::to_value(malformed).unwrap()["bases"],
        serde_json::json!([])
    );
}
