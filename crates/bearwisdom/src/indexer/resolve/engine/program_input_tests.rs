use super::*;

#[test]
fn serialized_global_evidence_preserves_missing_declarations_and_scope() {
    let input = Input {
        isolated: true,
        complete: true,
        roots: vec![],
        augmentations: vec![Part {
            unit: super::super::module_input::SourceModuleId(1),
            name: "Box".into(),
            binding: Some(7),
            declaration: None,
            kind: SymbolKind::Interface,
            type_space: true,
            parameters: vec!["T".into()],
            plain_parameters: false,
            members: vec!["first".into()],
            plain_merge: true,
            plain_header: true,
            type_heritage: false,
            surface: None,
        }],
        merge_rules: vec![(SymbolKind::Interface, SymbolKind::Interface)],
        types: None,
    };
    let stored = serde_json::to_value(&input).unwrap();
    let restored: Input = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), stored);
}
