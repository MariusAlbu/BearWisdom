use super::*;
use std::collections::HashSet;
use std::fs;
use tempfile::TempDir;

fn walked(root: &Path, relative_path: &str) -> WalkedFile {
    WalkedFile {
        relative_path: relative_path.to_owned(),
        absolute_path: root.join(relative_path),
        language: "typescript",
    }
}

#[test]
fn generic_secondary_scan_dispatches_to_the_ecosystem_adapter() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import { Client } from './generated/client';\n",
    )
    .unwrap();
    fs::write(
        root.join("src/generated/client.ts"),
        "export class Client {}\n",
    )
    .unwrap();

    let extra = crate::indexer::secondary_scan::pull_gitignored_imports(
        root,
        &[walked(root, "src/app.ts")],
    );

    assert_eq!(extra.len(), 1, "adapter scan produced {extra:?}");
    assert_eq!(extra[0].relative_path, "ext:gen:src/generated/client.ts");
    assert_eq!(extra[0].language, "typescript");
}

#[test]
fn external_demand_dispatch_preserves_decline_full_and_filter_results() {
    let mut demand = DemandSet::new();
    demand.add("react", "useState");
    let ambient = HashSet::new();

    match external_demand("ext:ts:react/index.d.ts", &demand, &ambient) {
        ExternalDemandDecision::Filter(symbols) => {
            assert_eq!(symbols, demand.for_module("react").unwrap());
        }
        _ => panic!("npm-owned package with demand must retain its filter"),
    }

    assert!(matches!(
        external_demand("ext:ts:__ts_lib__/lib.dom.d.ts", &demand, &ambient),
        ExternalDemandDecision::Full
    ));
    assert!(matches!(
        external_demand("ext:go:example.com/lib/source.go", &demand, &ambient),
        ExternalDemandDecision::Decline
    ));
}
