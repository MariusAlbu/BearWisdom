use super::*;

fn unit(id: u32, parent: u32, kind: &str) -> InputUnit {
    serde_json::from_value(serde_json::json!({ "id": id, "parent": parent, "exports": [], "imports": [], "stars": [], "wildcard_exclusions": [],
        "source_scope": { "kind": kind, "lexical_scope": id, "range": { "start": 0, "end": 40 }, "body": { "start": 10, "end": 40 },
            "ambient": true, "complete": true, "container_valid": true } })).unwrap()
}

#[test]
fn contribution_context_requires_direct_literal_or_configured_file_ownership() {
    let mut input = ModuleInput {
        globals: Some(Default::default()),
        units: vec![unit(1, 0, "Augmentation")],
        ..Default::default()
    };
    assert!(!valid(&input, false));
    assert!(valid(&input, true));
    input.units = vec![unit(1, 0, "Literal"), unit(2, 1, "Augmentation")];
    assert!(valid(&input, false));
    input.units[0].source_scope.as_mut().unwrap().kind = Kind::Namespace;
    assert!(!valid(&input, true));
    input.units[0].source_scope.as_mut().unwrap().kind = Kind::Literal;
    input.units[1]
        .source_scope
        .as_mut()
        .unwrap()
        .container_valid = false;
    assert!(!valid(&input, true));
}

#[test]
fn duplicate_dangling_and_incomplete_owners_cannot_attest_global_contributions() {
    for fault in 0..4 {
        let mut input = ModuleInput {
            globals: Some(Default::default()),
            units: vec![unit(1, 0, "Literal"), unit(2, 1, "Augmentation")],
            ..Default::default()
        };
        match fault {
            0 => input.units.push(unit(1, 0, "Literal")),
            1 => input.units[1].parent = SourceModuleId(99),
            2 => input.units[0].source_scope.as_mut().unwrap().complete = false,
            _ => input.units[0].parent = SourceModuleId(2),
        }
        assert!(!valid(&input, true), "fault {fault}");
    }
}
