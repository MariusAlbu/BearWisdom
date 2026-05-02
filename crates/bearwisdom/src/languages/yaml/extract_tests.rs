use super::*;

#[test]
fn top_level_keys_become_fields() {
    let src = "name: CI\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n";
    let r = extract(src, "/a/.github/workflows/ci.yml");
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"name"));
    assert!(names.contains(&"on"));
    assert!(names.contains(&"jobs"));
    // Nested keys (build, runs-on) are NOT surfaced.
    assert!(!names.contains(&"build"));
    assert!(!names.contains(&"runs-on"));
}

#[test]
fn comments_and_indent_ignored() {
    let src = "# header comment\nname: CI\n  nested: true\n";
    let r = extract(src, "ci.yml");
    let keys: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(keys, vec!["name"]);
}

#[test]
fn local_uses_path_emits_imports_ref() {
    let src = "jobs:\n  build:\n    steps:\n      - uses: ./.github/actions/setup\n";
    let r = extract(src, ".github/workflows/ci.yml");
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("expected an Imports ref");
    assert_eq!(imp.target_name, "./.github/actions/setup");
}

#[test]
fn external_uses_ref_is_skipped() {
    let src = "jobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n";
    let r = extract(src, ".github/workflows/ci.yml");
    assert!(
        r.refs.iter().all(|r| r.kind != EdgeKind::Imports),
        "external action ref should not be emitted"
    );
}

#[test]
fn uses_outside_gha_path_is_ignored() {
    // `uses:` is a GHA-specific keyword. In a docker-compose file or a
    // Helm values.yaml the same key can mean something else; we don't
    // try to extract it there.
    let src = "uses: ./not-a-real-action\n";
    let r = extract(src, "docker-compose.yml");
    assert!(
        r.refs.iter().all(|r| r.kind != EdgeKind::Imports),
        "uses: outside GHA path should not become an Imports ref"
    );
}

#[test]
fn reusable_workflow_with_yml_extension_resolves() {
    let src = "jobs:\n  call:\n    uses: ./.github/workflows/reusable.yml\n";
    let r = extract(src, ".github/workflows/main.yml");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "./.github/workflows/reusable.yml");
}

#[test]
fn parent_relative_uses_path_works() {
    let src = "      - uses: ../shared/compose-action\n";
    let r = extract(src, ".github/workflows/ci.yml");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "../shared/compose-action");
}

#[test]
fn quoted_uses_value_is_unwrapped() {
    let src = "      - uses: \"./.github/actions/lint\"\n";
    let r = extract(src, ".github/workflows/ci.yml");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "./.github/actions/lint");
}

#[test]
fn inline_comment_after_uses_is_stripped() {
    let src = "      - uses: ./.github/actions/setup # local helper\n";
    let r = extract(src, ".github/workflows/ci.yml");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "./.github/actions/setup");
}

#[test]
fn action_yml_root_is_recognised_as_gha() {
    // composite action definitions sit at `<repo>/action.yml` (not
    // under `.github/workflows/`). The detector should still allow
    // `uses:` extraction in those files for any nested step refs.
    let src = "name: my-action\nruns:\n  using: composite\n  steps:\n    - uses: ./other-action\n";
    let r = extract(src, "action.yml");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports);
    assert!(imp.is_some(), "expected uses: extracted for action.yml at repo root");
}
