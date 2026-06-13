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

/// Write Go's `vendor/modules.txt` inventory under `<root>/<rel>` (the `vendor`
/// dir lives at `<root>/<rel>`), the self-declaration of a Go vendored tree.
fn write_go_modules_txt(root: &Path, vendor_rel: &str) {
    let dir = root.join(vendor_rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("modules.txt"), "# github.com/x/y v1.0.0\n").unwrap();
}

/// Write Composer's `vendor/composer/installed.json` ledger under the `vendor`
/// dir at `<root>/<vendor_rel>`, the self-declaration of a Composer tree.
fn write_composer_installed_json(root: &Path, vendor_rel: &str) {
    let dir = root.join(vendor_rel).join("composer");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("installed.json"), "{\"packages\":[]}\n").unwrap();
}

/// Write a project root `go.mod` declaring `module`.
fn write_go_mod(root: &Path, module: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("go.mod"), format!("module {module}\n\ngo 1.22\n")).unwrap();
}

/// Write an empty `rebar.config` under `<root>/<rel>` (root when `rel` is "").
fn write_rebar_config(root: &Path, rel: &str) {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("rebar.config"), "{deps, []}.\n").unwrap();
}

/// Write `src/<pkg>.app.src` (the OTP application resource file) under
/// `<root>/<rel>`, the self-declaration of an erlang application.
fn write_app_src(root: &Path, rel: &str, pkg: &str) {
    let dir = root.join(rel).join("src");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{pkg}.app.src")),
        format!("{{application, {pkg}, []}}.\n"),
    )
    .unwrap();
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

// --- rebar3 `deps/` self-declaring vendored applications ---------------------

// (a) Erlang host (root rebar.config) with a `deps/<pkg>` carrying its own
//     `src/<pkg>.app.src` → that subtree is classified vendored-external.
#[test]
fn rebar_host_vendored_dep_with_app_src_is_external() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_rebar_config(root, ""); // host declares the rebar ecosystem
    write_app_src(root, "deps/cowboy", "cowboy");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);

    assert_eq!(prefixes, vec!["deps/cowboy".to_string()]);
    assert!(is_under_self_declared_vendor("deps/cowboy/src/cowboy.erl", &prefixes));
}

// A `deps/<pkg>` declaring itself via its own `rebar.config` (rather than an
// `.app.src`) is equally vendored.
#[test]
fn rebar_host_vendored_dep_with_rebar_config_is_external() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_rebar_config(root, "");
    write_rebar_config(root, "deps/jsx");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert_eq!(prefixes, vec!["deps/jsx".to_string()]);
}

// (b) Umbrella member under `apps/<name>/` self-declares via `.app.src` but is
//     a first-party project app — it must stay internal.
#[test]
fn rebar_umbrella_app_member_stays_internal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_rebar_config(root, "");
    write_app_src(root, "apps/myapp", "myapp");
    write_app_src(root, "lib/other", "other"); // the `lib/` app-dir variant too

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}

// (c) Non-erlang host (no root rebar.config) with a directory literally named
//     `deps/` full of erlang-looking files → NOT classified. The host-ecosystem
//     gate declines.
#[test]
fn non_rebar_host_with_deps_dir_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // No root rebar.config — the host does not declare the rebar ecosystem.
    write_app_src(root, "deps/cowboy", "cowboy");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}

// (d) A `deps/<pkg>` WITHOUT its own erlang manifest is not self-declaring —
//     not classified even under a rebar host.
#[test]
fn rebar_dep_without_own_manifest_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_rebar_config(root, "");
    // A bare directory under deps/ with a source file but no manifest.
    let dir = root.join("deps/loose/src");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("loose.erl"), "-module(loose).\n").unwrap();

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}

// --- Go vendoring (`vendor/modules.txt`) -------------------------------------

// A nested `vendor/` carrying Go's `modules.txt` inventory is a vendored
// dependency tree — the whole subtree is classified external.
#[test]
fn go_nested_vendor_with_modules_txt_is_external() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_go_modules_txt(root, "services/api/vendor");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert_eq!(prefixes, vec!["services/api/vendor".to_string()]);
    assert!(is_under_self_declared_vendor(
        "services/api/vendor/github.com/x/y/y.go",
        &prefixes
    ));
}

// The project's own module (its root `go.mod`) is never under a `vendor/`
// segment, so it stays internal even when a vendored tree exists.
#[test]
fn go_project_own_module_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_go_mod(root, "github.com/me/app");
    write_go_modules_txt(root, "services/api/vendor");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(!is_under_self_declared_vendor("go.mod", &prefixes));
    assert!(!is_under_self_declared_vendor("services/api/main.go", &prefixes));
    assert!(is_under_self_declared_vendor(
        "services/api/vendor/github.com/x/y/y.go",
        &prefixes
    ));
}

// A `vendor/` directory WITHOUT the `modules.txt` inventory is not a
// self-declaring Go vendored tree — not classified.
#[test]
fn go_vendor_without_modules_txt_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = root.join("services/api/vendor/foo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("foo.go"), "package foo\n").unwrap();

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}

// --- Composer vendoring (`vendor/composer/installed.json`) -------------------

// A nested `vendor/` carrying Composer's `composer/installed.json` ledger is a
// vendored dependency tree — the whole subtree is classified external.
#[test]
fn composer_nested_vendor_with_installed_json_is_external() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_composer_installed_json(root, "packages/admin/vendor");

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert_eq!(prefixes, vec!["packages/admin/vendor".to_string()]);
    assert!(is_under_self_declared_vendor(
        "packages/admin/vendor/doctrine/inflector/src/Inflector.php",
        &prefixes
    ));
}

// A `vendor/` directory WITHOUT the Composer ledger is not a self-declaring
// Composer vendored tree — not classified.
#[test]
fn composer_vendor_without_installed_json_is_not_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = root.join("packages/admin/vendor/acme/widget");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Widget.php"), "<?php\n").unwrap();

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}

// Precision guard: a normal source tree (`src/`) with no in-subtree vendor
// ledger and no foreign manifest is never classified as vendored.
#[test]
fn plain_src_tree_is_never_vendored() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = root.join("src/services");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.go"), "package main\n").unwrap();
    fs::write(dir.join("handler.php"), "<?php\n").unwrap();

    let prefixes = self_declared_vendor_prefixes(root, &[], None);
    assert!(prefixes.is_empty());
}
