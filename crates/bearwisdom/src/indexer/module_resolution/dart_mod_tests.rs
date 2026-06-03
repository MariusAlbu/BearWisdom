use super::*;

fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
    DartModuleResolver::new(Some("app".to_string())).resolve_to_file(spec, from, files)
}

#[test]
fn relative_dart_import_resolves() {
    let files = &["lib/x.dart", "lib/main.dart"];
    assert_eq!(
        resolve("./x.dart", "lib/main.dart", files),
        Some("lib/x.dart".into())
    );
}

#[test]
fn relative_parent_dir_resolves() {
    let files = &["lib/util.dart"];
    assert_eq!(
        resolve("../util.dart", "lib/src/widget.dart", files),
        Some("lib/util.dart".into())
    );
}

#[test]
fn bare_neighbour_resolves() {
    let files = &["lib/helpers.dart", "lib/main.dart"];
    assert_eq!(
        resolve("helpers.dart", "lib/main.dart", files),
        Some("lib/helpers.dart".into())
    );
}

#[test]
fn package_self_resolves() {
    let files = &["lib/x.dart", "lib/main.dart"];
    assert_eq!(
        resolve("package:app/x.dart", "lib/main.dart", files),
        Some("lib/x.dart".into())
    );
}

#[test]
fn package_self_nested_resolves() {
    let files = &["lib/src/models/user.dart"];
    assert_eq!(
        resolve("package:app/src/models/user.dart", "lib/main.dart", files),
        Some("lib/src/models/user.dart".into())
    );
}

#[test]
fn external_declines() {
    let files = &["lib/x.dart"];
    assert!(resolve("dart:async", "lib/main.dart", files).is_none());
    assert!(resolve("package:flutter/material.dart", "lib/main.dart", files).is_none());
}

#[test]
fn package_self_declines_without_name() {
    // With no self-package name plumbed, `package:` cannot be distinguished
    // from a foreign package — decline and leave it to classify_external.
    let files = &["lib/x.dart"];
    assert!(DartModuleResolver::new(None)
        .resolve_to_file("package:app/x.dart", "lib/main.dart", files)
        .is_none());
}
