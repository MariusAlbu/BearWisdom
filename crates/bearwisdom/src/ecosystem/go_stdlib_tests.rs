use super::*;
use std::fs;

fn write_pkg_file(dir: &Path, file_name: &str, contents: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(file_name), contents).unwrap();
}

#[test]
fn ecosystem_identity() {
    let e = GoStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["go"]);
}

#[test]
fn legacy_locator_tag() {
    assert_eq!(
        ExternalSourceLocator::ecosystem(&GoStdlibEcosystem),
        "go-stdlib"
    );
}

/// `net` and `net/http` are sibling packages sharing a filesystem prefix —
/// each must get its own root keyed by its exact import path, not one root
/// covering both.
#[test]
fn collect_package_roots_keys_nested_packages_separately() {
    let tmp = std::env::temp_dir().join(format!("bw-go-stdlib-test-{}", std::process::id()));
    let src = tmp.join("src");
    write_pkg_file(&src.join("net"), "net.go", "package net\n");
    write_pkg_file(&src.join("net").join("http"), "server.go", "package http\n");
    write_pkg_file(
        &src.join("net").join("http").join("httptest"),
        "httptest.go",
        "package httptest\n",
    );
    // Excluded directories: internal tooling and non-source dirs.
    write_pkg_file(&src.join("net").join("internal"), "foo.go", "package internal\n");
    write_pkg_file(&src.join("net").join("testdata"), "bar.go", "package testdata\n");
    write_pkg_file(&src.join("cmd").join("go"), "main.go", "package main\n");

    let mut roots = Vec::new();
    collect_package_roots(&src, &src, &mut roots, 0);
    fs::remove_dir_all(&tmp).ok();

    let module_paths: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(module_paths.contains(&"net"), "{module_paths:?}");
    assert!(module_paths.contains(&"net/http"), "{module_paths:?}");
    assert!(module_paths.contains(&"net/http/httptest"), "{module_paths:?}");
    assert!(
        !module_paths.iter().any(|m| m.contains("internal")),
        "internal package must not get its own root: {module_paths:?}"
    );
    assert!(
        !module_paths.iter().any(|m| m.contains("testdata")),
        "testdata must not get its own root: {module_paths:?}"
    );
    assert!(
        !module_paths.iter().any(|m| m.starts_with("cmd")),
        "cmd tooling must not get its own root: {module_paths:?}"
    );

    let http_root = roots.iter().find(|r| r.module_path == "net/http").unwrap();
    assert_eq!(http_root.requested_imports, vec!["net/http".to_string()]);
    assert_eq!(http_root.root, src.join("net").join("http"));
}

/// Each root's walk is single-directory: a parent package's file list must
/// not include a nested sibling package's files.
#[test]
fn walk_go_tree_does_not_recurse_into_sibling_packages() {
    let tmp = std::env::temp_dir().join(format!("bw-go-stdlib-walk-test-{}", std::process::id()));
    let net_dir = tmp.join("net");
    write_pkg_file(&net_dir, "net.go", "package net\n");
    write_pkg_file(&net_dir.join("http"), "server.go", "package http\n");

    let dep = ExternalDepRoot {
        module_path: "net".to_string(),
        version: String::new(),
        root: net_dir.clone(),
        ecosystem: "go-stdlib",
        package_id: None,
        requested_imports: vec!["net".to_string()],
    };
    let files = walk_go_tree(&dep);
    fs::remove_dir_all(&tmp).ok();

    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].relative_path, "ext:go-stdlib/net/net.go");
}

/// The exact-match keying this task exists to fix: a ref carrying
/// `module: Some("net/http")` must hit `locate("net/http", name)` — and
/// must NOT be satisfiable under the sibling package's key `"net"`.
#[test]
fn build_symbol_index_locates_exact_package_match() {
    let tmp = std::env::temp_dir().join(format!("bw-go-stdlib-index-test-{}", std::process::id()));
    let src = tmp.join("src");
    write_pkg_file(&src.join("net"), "net.go", "package net\n\nfunc Dial() {}\n");
    write_pkg_file(
        &src.join("net").join("http"),
        "server.go",
        "package http\n\nfunc Get() {}\n",
    );

    let mut roots = Vec::new();
    collect_package_roots(&src, &src, &mut roots, 0);
    let index = super::super::go_mod::build_go_symbol_index(&roots);
    fs::remove_dir_all(&tmp).ok();

    assert!(
        index.locate("net/http", "Get").is_some(),
        "exact package match must hit"
    );
    assert!(
        index.locate("net", "Get").is_none(),
        "a sibling package's export must not leak into the parent package's key"
    );
    assert!(index.locate("net", "Dial").is_some());

    // Fallback-by-name still works for unqualified lookups (chain-walker
    // bail-outs that don't know which module a name lives in).
    let hits = index.find_by_name("Get");
    assert!(
        hits.iter().any(|(module, _)| *module == "net/http"),
        "{hits:?}"
    );
}
