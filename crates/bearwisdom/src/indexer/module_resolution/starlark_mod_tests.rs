use super::*;

fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
    StarlarkModuleResolver::new().resolve_to_file(spec, from, files)
}

#[test]
fn absolute_label_resolves() {
    let files = &["pkg/defs.bzl", "pkg/BUILD"];
    assert_eq!(
        resolve("//pkg:defs.bzl", "pkg/BUILD", files),
        Some("pkg/defs.bzl".into())
    );
}

#[test]
fn nested_package_label_resolves() {
    let files = &["a/b/c/rules.bzl"];
    assert_eq!(
        resolve("//a/b/c:rules.bzl", "a/BUILD", files),
        Some("a/b/c/rules.bzl".into())
    );
}

#[test]
fn label_resolves_against_repo_root_prefix() {
    // A monorepo checkout where the indexed paths carry a leading workspace
    // directory: the suffix match still lands the file.
    let files = &["workspace/tools/build_defs/lib.bzl"];
    assert_eq!(
        resolve("//tools/build_defs:lib.bzl", "workspace/BUILD", files),
        Some("workspace/tools/build_defs/lib.bzl".into())
    );
}

#[test]
fn same_package_relative_label_resolves() {
    let files = &["pkg/local.bzl", "pkg/BUILD"];
    assert_eq!(
        resolve(":local.bzl", "pkg/BUILD", files),
        Some("pkg/local.bzl".into())
    );
}

#[test]
fn foreign_repo_label_declines() {
    // `@`-prefixed labels reference another repository — external.
    let files = &["pkg/defs.bzl"];
    assert!(resolve("@rules_cc//cc:defs.bzl", "pkg/BUILD", files).is_none());
}

#[test]
fn unknown_label_declines() {
    let files = &["pkg/defs.bzl"];
    assert!(resolve("//other:missing.bzl", "pkg/BUILD", files).is_none());
}

#[test]
fn empty_specifier_declines() {
    let files = &["pkg/defs.bzl"];
    assert!(resolve("", "pkg/BUILD", files).is_none());
}

#[test]
fn windows_separators_normalise() {
    let files = &["pkg\\sub\\defs.bzl"];
    assert_eq!(
        resolve("//pkg/sub:defs.bzl", "pkg/BUILD", files),
        Some("pkg/sub/defs.bzl".into())
    );
}
