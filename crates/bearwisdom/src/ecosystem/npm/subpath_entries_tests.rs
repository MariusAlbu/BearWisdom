use std::path::PathBuf;

use super::resolve_package_subpath_entries;
use crate::ecosystem::externals::ExternalDepRoot;

fn mkdep(root: PathBuf, name: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: name.to_string(),
        version: String::new(),
        root,
        ecosystem: super::super::LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// `preact/hooks` shape — a concrete subpath export ships its own .d.ts entry
/// the package-root walk never reaches. The `.` root and `./*` wildcard keys
/// are skipped; only hand-declared concrete subpaths return.
#[test]
fn concrete_exports_subpaths_resolve_and_root_and_wildcards_are_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("preact");
    std::fs::create_dir_all(root.join("hooks").join("src")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name":"preact",
          "exports":{
            ".":{"types":"./src/index.d.ts"},
            "./hooks":{"types":"./hooks/src/index.d.ts"},
            "./compat/*":{"types":"./compat/src/*.d.ts"}
          }
        }"#,
    )
    .unwrap();
    std::fs::write(root.join("src").join("index.d.ts"), "export const h: 1;").unwrap();
    std::fs::write(
        root.join("hooks").join("src").join("index.d.ts"),
        "export function useState<T>(v: T): [T];",
    )
    .unwrap();

    let dep = mkdep(root.clone(), "preact");
    let entries = resolve_package_subpath_entries(&dep);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "/hooks");
    assert_eq!(
        entries[0].1,
        root.join("hooks").join("src").join("index.d.ts")
    );
}

/// A single-segment flat-file subpath with no `exports` map resolves to the
/// sibling declaration file.
#[test]
fn demanded_subpath_resolves_flat_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export const x: 1;").unwrap();
    std::fs::write(root.join("server.d.ts"), "export declare class Req {}").unwrap();

    let mut dep = mkdep(root.clone(), "pkg");
    dep.requested_imports = vec!["pkg/server".to_string()];
    let entries = resolve_package_subpath_entries(&dep);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "/server");
    assert_eq!(entries[0].1, root.join("server.d.ts"));
}

/// A nested demanded subpath is a flat-file entry shape too — the probe must
/// not stop at one segment.
#[test]
fn nested_demanded_subpath_resolves_flat_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("pkg");
    std::fs::create_dir_all(root.join("legacy")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export const x: 1;").unwrap();
    std::fs::write(
        root.join("legacy").join("image.d.ts"),
        "export declare const Legacy: { src: string };",
    )
    .unwrap();

    let mut dep = mkdep(root.clone(), "pkg");
    dep.requested_imports = vec!["pkg/legacy/image".to_string()];
    let entries = resolve_package_subpath_entries(&dep);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "/legacy/image");
    assert_eq!(entries[0].1, root.join("legacy").join("image.d.ts"));
}

/// A nested demanded subpath that names a directory resolves through its
/// `index.d.ts`.
#[test]
fn nested_demanded_subpath_resolves_directory_index() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("pkg");
    std::fs::create_dir_all(root.join("font").join("google")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export const x: 1;").unwrap();
    std::fs::write(
        root.join("font").join("google").join("index.d.ts"),
        "export declare function Inter(): void;",
    )
    .unwrap();

    let mut dep = mkdep(root.clone(), "pkg");
    dep.requested_imports = vec!["pkg/font/google".to_string()];
    let entries = resolve_package_subpath_entries(&dep);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "/font/google");
    assert_eq!(
        entries[0].1,
        root.join("font").join("google").join("index.d.ts")
    );
}

/// A specifier carrying `.`/`..` segments names no entry — the probe must
/// never leave the dep root.
#[test]
fn traversal_segments_are_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export const x: 1;").unwrap();
    std::fs::write(tmp.path().join("passwd.d.ts"), "export const leak: 1;").unwrap();
    std::fs::write(root.join("x.d.ts"), "export const x2: 1;").unwrap();

    let mut dep = mkdep(root, "pkg");
    dep.requested_imports = vec![
        "pkg/../../passwd".to_string(),
        "pkg/./x".to_string(),
        "pkg/".to_string(),
    ];
    assert!(resolve_package_subpath_entries(&dep).is_empty());
}

/// A subpath already declared in the `exports` map is authoritative — the
/// demand probe must not add a second entry for the same suffix.
#[test]
fn exports_map_key_wins_over_probe() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("pkg");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name":"pkg",
          "exports":{".":{"types":"./index.d.ts"},"./sub":{"types":"./dist/sub.d.ts"}}
        }"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export const x: 1;").unwrap();
    std::fs::write(root.join("dist").join("sub.d.ts"), "export const a: 1;").unwrap();
    // A same-named sibling at the root the probe would otherwise pick up.
    std::fs::write(root.join("sub.d.ts"), "export const b: 1;").unwrap();

    let mut dep = mkdep(root.clone(), "pkg");
    dep.requested_imports = vec!["pkg/sub".to_string()];
    let entries = resolve_package_subpath_entries(&dep);
    assert_eq!(entries.len(), 1, "one entry per suffix: {entries:?}");
    assert_eq!(entries[0].1, root.join("dist").join("sub.d.ts"));
}

/// The demand gate is what keeps the probe bounded — a declaration file nobody
/// imports produces no entry.
#[test]
fn undemanded_subpath_is_not_probed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export const x: 1;").unwrap();
    std::fs::write(root.join("other.d.ts"), "export const other: 1;").unwrap();

    let dep = mkdep(root, "pkg");
    assert!(resolve_package_subpath_entries(&dep).is_empty());
}
