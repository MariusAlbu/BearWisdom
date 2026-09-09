use super::*;

#[test]
fn configuration_round_trip_preserves_explicit_absence_and_conditions() {
    let record = ModulePackage {
        root: "consumer".into(),
        name: "consumer".into(),
        fingerprint: "hash".into(),
        targets: vec![ModuleTarget {
            name: "api".into(),
            path: "custom/entry.rs".into(),
            kind: TargetKind::Library,
            conditional: false,
        }],
        dependencies: vec![ModuleDependency {
            alias: "dep".into(),
            package: "real".into(),
            root: None,
            renamed: true,
            kind: TargetKind::Development,
            conditional: true,
        }],
    };
    assert_eq!(
        record,
        serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap()
    );
}
