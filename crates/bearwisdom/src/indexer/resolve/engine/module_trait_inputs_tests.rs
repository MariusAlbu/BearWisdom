use super::*;

#[test]
fn durable_trait_contract_distinguishes_implementation_binders_from_declaration_ids() {
    let header = Header {
        owner: Owner::Implementation(12),
        declaration: Some(49),
        self_binding: 12,
        unit: SourceModuleId(3),
        scope: 7,
        span: SourceSpan {
            start: 101,
            end: 201,
        },
        members: vec![31, 32],
        enabled: true,
        negative: false,
        parameters: vec![
            (14, GenericParamKind::Lifetime),
            (15, GenericParamKind::Type),
        ],
    };
    let json = serde_json::to_string(&header).unwrap();
    let cold: Header = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&cold).unwrap(), json);
    assert_ne!(cold.owner, Owner::Declaration(12));
    assert_eq!(cold.parameters[1], (15, GenericParamKind::Type));
}
