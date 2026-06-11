use super::*;
use std::fs;
use std::path::Path;

/// An npm `PackageInfo` rooted at `path` (folder-derived key irrelevant here).
fn npm_pkg(path: &str, declared_name: &str) -> PackageInfo {
    PackageInfo {
        id: None,
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        kind: Some("npm".to_string()),
        manifest: Some(format!("{path}/package.json")),
        declared_name: Some(declared_name.to_string()),
        is_publishable: true,
    }
}

/// Write `package.json` declaring `name` under `<root>/<rel>`.
fn write_package_json(root: &Path, rel: &str, name: &str) {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("package.json"),
        format!("{{\"name\":\"{name}\"}}"),
    )
    .unwrap();
}

/// Write `bower.json` declaring `name` under `<root>/<rel>`.
fn write_bower_json(root: &Path, rel: &str, name: &str) {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("bower.json"), format!("{{\"name\":\"{name}\"}}")).unwrap();
}

// (a) Java-shaped host: no root package.json, a deep self-declaring npm subtree
//     → that subtree is classified vendored-external.
#[test]
fn java_host_vendored_npm_subtree_is_external() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Host owns no npm — only the deep vendored manifest exists on disk.
    let vendored = "src/main/resources/static/admin/plugins/editormd";
    write_package_json(root, vendored, "editor.md");

    // The recursive manifest scan registers the deep package.json as npm.
    let packages = vec![npm_pkg(vendored, "editor.md")];
    let prefixes = self_declared_vendor_prefixes(root, &packages, None);

    assert_eq!(prefixes, vec![vendored.to_string()]);
    assert!(is_under_self_declared_vendor(
        &format!("{vendored}/editormd.js"),
        &prefixes
    ));
    assert!(!is_under_self_declared_vendor("src/main/java/App.java", &prefixes));
}

// A depth-1 npm subproject of a non-JS host (a `frontend/` SPA) is plausibly
// first-party — the depth guard keeps it internal rather than mislabeling it
// vendored.
#[test]
fn depth_one_npm_subproject_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let packages = vec![npm_pkg("frontend", "myblog-frontend")];
    let prefixes = self_declared_vendor_prefixes(root, &packages, None);
    assert!(prefixes.is_empty());
}

// (b) JS monorepo: root package.json declares workspaces → nested member is NOT
//     classified. The host owns npm via the workspace; the gate declines.
#[test]
fn js_monorepo_member_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let packages = vec![
        npm_pkg("packages/ui", "@org/ui"),
        npm_pkg("packages/core", "@org/core"),
    ];
    let prefixes = self_declared_vendor_prefixes(root, &packages, Some("pnpm-workspace"));
    assert!(prefixes.is_empty());
}

// (c) Plain (non-monorepo) JS project with a root package.json → nothing
//     classified. Conservative gate: the host owns npm at the root.
#[test]
fn root_npm_project_classifies_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let packages = vec![
        npm_pkg("", "my-app"),
        // A nested package.json in a plain JS project (e.g. an example app)
        // stays internal because the host owns npm.
        npm_pkg("examples/demo", "my-app-demo"),
    ];
    let prefixes = self_declared_vendor_prefixes(root, &packages, None);
    assert!(prefixes.is_empty());
}

// (d) Root npm manifest present (path "."), declared name irrelevant — the
//     root-ownership signal alone declines the gate.
#[test]
fn root_dot_path_npm_declines_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let packages = vec![npm_pkg(".", "rootish"), npm_pkg("vendor/lib", "foreign")];
    let prefixes = self_declared_vendor_prefixes(root, &packages, None);
    assert!(prefixes.is_empty());
}

// bower.json-only subtree (no sibling package.json) is invisible to the npm
// manifest scan but IS self-declaring — discovered by the bower walk.
#[test]
fn bower_only_subtree_is_classified_for_non_js_host() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let vendored = "src/static/plugins/widget";
    write_bower_json(root, vendored, "some-widget");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert_eq!(prefixes, vec![vendored.to_string()]);
}

// A bower.json that sits beside a package.json is already covered by the npm
// path — it must not be double-counted by the bower walk.
#[test]
fn bower_beside_package_json_not_double_counted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let vendored = "src/static/plugins/editor";
    write_package_json(root, vendored, "editor.md");
    write_bower_json(root, vendored, "editor.md");

    let packages = vec![npm_pkg(vendored, "editor.md")];
    let prefixes = self_declared_vendor_prefixes(root, &packages, None);
    assert_eq!(prefixes, vec![vendored.to_string()]);
}

// Depth guard: a root-level or first-level bower.json is the host's own, not
// vendored — it must not be classified.
#[test]
fn shallow_bower_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_bower_json(root, "", "host-app"); // depth 0 (root)
    write_bower_json(root, "frontend", "host-frontend"); // depth 1

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}

#[test]
fn under_vendor_matches_subtree_not_prefix_sibling() {
    let prefixes = vec!["src/plugins/editormd".to_string()];
    assert!(is_under_self_declared_vendor(
        "src/plugins/editormd/lib/x.js",
        &prefixes
    ));
    assert!(is_under_self_declared_vendor("src/plugins/editormd", &prefixes));
    assert!(!is_under_self_declared_vendor(
        "src/plugins/editormd-extra/x.js",
        &prefixes
    ));
    assert!(!is_under_self_declared_vendor("src/app/main.js", &prefixes));
}

#[test]
fn windows_backslash_paths_normalize() {
    let prefixes = vec!["src/plugins/editormd".to_string()];
    assert!(is_under_self_declared_vendor(
        "src\\plugins\\editormd\\lib\\x.js",
        &prefixes
    ));
}

#[test]
fn empty_prefix_set_matches_nothing() {
    assert!(!is_under_self_declared_vendor("anything/at/all.js", &[]));
}
