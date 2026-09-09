use super::*;

#[test]
fn real_project_class_receiver_cascades_survive_cold_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut value = super::super::tests::fixture(dir.path());
    let source = "export class Doc {\n  touch() {}\n}\nexport class Service<T> {\n  doc: Doc;\n  next(): Doc { return this.doc; }\n  run() { this.next().touch(); this.doc.touch(); }\n}\n";
    std::fs::write(dir.path().join("a.ts"), source).unwrap();
    value["files"][0]["sha256"] = format!("{:x}", Sha256::digest(source)).into();
    value["calls"] = serde_json::json!([
        {"site":{"file":1,"byte_offset":source.find("next().").unwrap(),"kind":"calls"},"target":{"file":1,"line":5,"col":2,"kind":"method"}},
        {"site":{"file":1,"byte_offset":source.find("touch();").unwrap(),"kind":"calls"},"target":{"file":1,"line":1,"col":2,"kind":"method"}},
        {"site":{"file":1,"byte_offset":source.rfind("touch();").unwrap(),"kind":"calls"},"target":{"file":1,"line":1,"col":2,"kind":"method"}}
    ]);
    let path = dir.path().join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let report = evaluate_manifest(&path).unwrap();
    assert_eq!(report.binding_mode, ProjectBindingMode::Legacy);
    assert_eq!(report.fresh.counts.correct, 3, "{:?}", report.fresh);
    assert_eq!(report.cold.counts.correct, 3, "{:?}", report.cold);
    assert_eq!(report.fresh, report.cold);
    assert!(report.snapshot_changes.is_empty());
}

#[test]
fn configured_project_runner_binds_split_globals_and_honors_source_isolation() {
    assert_configured_split_source(0);
}

#[test]
fn configured_project_runner_keeps_large_provider_callback_and_return_cascades() {
    assert_configured_split_source(crate::indexer::flow::MAX_FLOW_SOURCE_BYTES + 1);
}

fn assert_configured_split_source(padding: usize) {
    let dir = tempfile::tempdir().unwrap();
    let mut value = super::super::tests::fixture(dir.path());
    let a = "interface Catalog<T> { first(): T; }";
    let padded = format!(
        "{}interface Catalog<T> {{ each(callback: (item: T) => void): void; }}",
        " ".repeat(padding)
    );
    let b = padded.as_str();
    let main = "export {}; class Doc { touch() {} } function run(list: Catalog<Doc>) { list.first().touch(); list.each(item => item.touch()); }";
    value["version"] = 2.into();
    value["files"] = serde_json::Value::Array([(a,"a.d.ts","Syntax"),(b,"b.d.ts","Syntax"),(main,"main.ts","Module")]
        .iter().enumerate().map(|(i, &(source, name, scope))| {
            let path = dir.path().join(name); std::fs::write(&path, source).unwrap();
            serde_json::json!({"id":i+1,"path":path,"index_path":name,"sha256":format!("{:x}",Sha256::digest(source)),
                "language":"typescript","selected":i==2,"source_scope":scope})
        }).collect());
    value["calls"] = serde_json::Value::Array(
        [
            (main.find("first()").unwrap(), 1, a.find("first()").unwrap()),
            (
                main.find("touch();").unwrap(),
                3,
                main.find("touch() {}").unwrap(),
            ),
            (main.find("each(").unwrap(), 2, b.find("each(").unwrap()),
            (
                main.rfind("touch()").unwrap(),
                3,
                main.find("touch() {}").unwrap(),
            ),
        ]
        .into_iter()
        .map(|(byte, file, col)| {
            serde_json::json!({"site":{"file":3,"byte_offset":byte,"kind":"calls"},
        "target":{"file":file,"line":0,"col":col,"kind":"method"}})
        })
        .collect(),
    );
    let path = dir.path().join("manifest.json");
    for (scope, expected) in [("Syntax", 4), ("Module", 2), ("Unknown", 0)] {
        value["files"][1]["source_scope"] = scope.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let report =
            evaluate_manifest_with_mode(&path, ProjectBindingMode::ConfiguredProgram).unwrap();
        assert_eq!(report.binding_mode, ProjectBindingMode::ConfiguredProgram);
        assert_eq!(
            report.configured_source_gaps.len(),
            usize::from(scope == "Unknown")
        );
        assert_eq!(
            report.fresh.counts.correct, expected,
            "{scope}: {:?}",
            report.fresh
        );
        assert_eq!(report.fresh.counts.incorrect, 0);
        assert_eq!(report.fresh, report.cold);
        assert!(report.snapshot_changes.is_empty());
        assert!(!report.gate_eligible);
    }
}

#[test]
fn real_project_runner_preserves_unlabelled_sites_and_fresh_cold_identity() {
    let dir = tempfile::tempdir().unwrap();
    let value = super::super::tests::fixture(dir.path());
    let path = dir.path().join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let before = std::fs::read(dir.path().join("a.ts")).unwrap();
    let report = evaluate_manifest(&path).unwrap();
    assert_eq!(report.compiler_calls, 2);
    assert_eq!(report.fresh.counts.correct, 1);
    assert_eq!(report.fresh.counts.unlabelled_observations, 1);
    assert_eq!(report.unlabelled_compiler_calls["no_compiler_target"], 1);
    assert_eq!(report.fresh, report.cold);
    assert!(report.snapshot_changes.is_empty());
    assert!(!report.gate_eligible);
    assert_eq!(report.compiler_sites_not_extracted, 0);
    assert_eq!(report.extractor_only_sites, 0);
    assert_eq!(std::fs::read(dir.path().join("a.ts")).unwrap(), before);
}
