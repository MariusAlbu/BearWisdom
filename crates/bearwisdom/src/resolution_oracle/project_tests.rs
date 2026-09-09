use super::*;

#[test]
fn kind_disagreements_do_not_relabel_wrong_targets_as_correct() {
    let site = ReferenceSite {
        file: FixtureFileId(1),
        byte_offset: 20,
        kind: EdgeKind::Calls,
    };
    let target = DeclarationSite {
        file: FixtureFileId(1),
        line: 1,
        col: 2,
        kind: SymbolKind::Function,
    };
    for (actual, count) in [
        (
            DeclarationSite {
                kind: SymbolKind::Variable,
                ..target
            },
            1,
        ),
        (
            DeclarationSite {
                line: 2,
                kind: SymbolKind::Variable,
                ..target
            },
            0,
        ),
        (
            DeclarationSite {
                file: FixtureFileId(2),
                ..target
            },
            0,
        ),
        (target, 0),
    ] {
        let report = evaluate(
            CorpusRevision([0; 32]),
            &[ExpectedReference {
                site,
                target: Some(target),
            }],
            &[Observation {
                site,
                binding: Some(ObservedBinding::Resolved(actual)),
            }],
        )
        .unwrap();
        assert_eq!(declaration_kind_disagreements(&report), count);
        assert_eq!(report.counts.correct, u64::from(actual == target));
        assert_eq!(report.counts.incorrect, u64::from(actual != target));
    }
}

pub(super) fn fixture(dir: &Path) -> serde_json::Value {
    let source = "function f() {} function run() { f(); missing(); }";
    let path = dir.join("a.ts");
    let config = dir.join("tsconfig.json");
    std::fs::write(&path, source).unwrap();
    std::fs::write(&config, "{}").unwrap();
    serde_json::json!({"version":1,"compiler":{"name":"TypeScript","version":"5.9.3"},"root":dir,"config":config,
        "selection":{"kind":"all_call_expressions","split":"development"},"compiler_options":{},
        "files":[{"id":1,"path":path,"index_path":"a.ts","sha256":format!("{:x}",Sha256::digest(source)),"language":"typescript","selected":true}],
        "inputs":[{"path":config,"sha256":format!("{:x}",Sha256::digest("{}"))}],
        "calls":[{"site":{"file":1,"byte_offset":source.rfind("f()").unwrap(),"kind":"calls"},"target":{"file":1,"line":0,"col":0,"kind":"function"}},
            {"site":{"file":1,"byte_offset":source.find("missing()").unwrap(),"kind":"calls"},"reason":"no_compiler_target"}],
        "diagnostics":[],"limitations":["development_cohort_not_held_out"]})
}

#[test]
fn real_project_manifest_rejects_missing_labels_duplicates_and_changed_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let good = fixture(dir.path());
    let manifest: ProjectManifest = serde_json::from_value(good.clone()).unwrap();
    manifest.validate().unwrap();
    manifest.verify_inputs().unwrap();
    for changed in [
        {
            let mut v = good.clone();
            v["version"] = 2.into();
            v
        },
        {
            let mut v = good.clone();
            v["calls"][1].as_object_mut().unwrap().remove("reason");
            v
        },
        {
            let mut v = good.clone();
            v["calls"][1] = v["calls"][0].clone();
            v
        },
        {
            let mut v = good.clone();
            v["calls"][0]["target"]["file"] = 2.into();
            v
        },
        {
            let mut v = good.clone();
            v["files"][0]["selected"] = false.into();
            v
        },
        {
            let mut v = good.clone();
            v["inputs"] = serde_json::json!([]);
            v
        },
        {
            let mut v = good.clone();
            v["source_binding_order"] = serde_json::json!([]);
            v
        },
        {
            let mut v = good.clone();
            v["source_binding_order"] = serde_json::json!([1, 1]);
            v
        },
        {
            let mut v = good.clone();
            v["source_binding_order"] = serde_json::json!([99]);
            v
        },
        {
            let mut v = good.clone();
            v["calls"][0]["reason"] = "".into();
            v
        },
    ] {
        assert!(serde_json::from_value::<ProjectManifest>(changed)
            .unwrap()
            .validate()
            .is_err());
    }
    for changed in [
        {
            let mut v = good.clone();
            v["calls"][0]["site"]["byte_offset"] = 99999.into();
            v
        },
        {
            let mut v = good.clone();
            v["calls"][0]["target"]["line"] = 99.into();
            v
        },
        {
            let mut v = good.clone();
            v["calls"][0]["target"]["col"] = 99999.into();
            v
        },
    ] {
        assert!(serde_json::from_value::<ProjectManifest>(changed)
            .unwrap()
            .verify_inputs()
            .is_err());
    }
    std::fs::write(dir.path().join("a.ts"), "function changed() {}").unwrap();
    assert!(manifest.verify_inputs().is_err());
}

#[test]
fn configured_project_requires_versioned_scope_evidence_for_every_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut value = fixture(dir.path());
    let path = dir.path().join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error =
        evaluate_manifest_with_mode(&path, ProjectBindingMode::ConfiguredProgram).unwrap_err();
    assert!(error.to_string().contains("source-scope evidence"));
    value["version"] = 2.into();
    assert!(serde_json::from_value::<ProjectManifest>(value.clone())
        .unwrap()
        .validate()
        .is_err());
    value["files"][0]["source_scope"] = "Syntax".into();
    serde_json::from_value::<ProjectManifest>(value.clone())
        .unwrap()
        .verify_inputs()
        .unwrap();
    value["version"] = 3.into();
    assert!(serde_json::from_value::<ProjectManifest>(value)
        .unwrap()
        .validate()
        .is_err());
}
