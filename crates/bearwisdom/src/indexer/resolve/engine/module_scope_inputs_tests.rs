use super::*;

#[test]
fn source_scope_evidence_roundtrips_without_equating_capture_with_program_legality() {
    let evidence = Evidence {
        kind: Kind::Augmentation,
        lexical_scope: 8,
        range: SourceSpan { start: 12, end: 40 },
        body: SourceSpan { start: 20, end: 40 },
        ambient: false,
        container_valid: true,
        complete: true,
    };
    let value = serde_json::to_value(&evidence).unwrap();
    let cold: Evidence = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(cold).unwrap(), value);
}
