// ---------------------------------------------------------------------------
// Tests (migrated from externals/dart.rs, uses internal parse_pubspec_deps)
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::discovery::discover_dart_externals_from_cache;
use super::reachability::extract_dart_exports;
use super::*;
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};

#[test]
fn ecosystem_identity() {
    let p = PubEcosystem;
    assert_eq!(p.id(), ID);
    assert_eq!(Ecosystem::kind(&p), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&p), &["dart"]);
}

#[test]
fn legacy_locator_tag_is_dart() {
    assert_eq!(ExternalSourceLocator::ecosystem(&PubEcosystem), "dart");
}

fn make_dart_fixture(root: &Path, deps: &[&str]) {
    std::fs::create_dir_all(root).unwrap();
    let mut pubspec = "name: test_app\ndependencies:\n".to_string();
    let dart_tool = root.join(".dart_tool");
    std::fs::create_dir_all(&dart_tool).unwrap();

    let cache_dir = root.parent().unwrap().join("_dart_pub_cache");
    let mut packages = Vec::new();
    for dep in deps {
        pubspec.push_str(&format!("  {dep}: ^1.0.0\n"));
        let pkg_dir = cache_dir.join(format!("{dep}-1.0.0"));
        let lib_dir = pkg_dir.join("lib");
        std::fs::create_dir_all(&lib_dir).unwrap();
        std::fs::write(lib_dir.join(format!("{dep}.dart")), format!("class {dep}Widget {{}}\n")).unwrap();
        std::fs::create_dir_all(lib_dir.join("src")).unwrap();
        std::fs::write(lib_dir.join("src").join("internal.dart"), "class _Internal {}\n").unwrap();

        let root_uri = format!("../../_dart_pub_cache/{dep}-1.0.0");
        packages.push(serde_json::json!({
            "name": dep,
            "rootUri": root_uri,
            "packageUri": "lib/",
            "version": "1.0.0"
        }));
    }
    std::fs::write(root.join("pubspec.yaml"), &pubspec).unwrap();
    let config = serde_json::json!({ "configVersion": 2, "packages": packages });
    std::fs::write(dart_tool.join("package_config.json"), config.to_string()).unwrap();
}

fn cleanup_dart(name: &str) {
    let tmp = std::env::temp_dir().join(name);
    let cache = std::env::temp_dir().join("_dart_pub_cache");
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn dart_discovers_declared_deps() {
    let tmp = std::env::temp_dir().join("bw-test-pub-discover");
    cleanup_dart("bw-test-pub-discover");
    make_dart_fixture(&tmp, &["http", "provider"]);

    let roots = discover_dart_externals(&tmp);
    let mut names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["http", "provider"]);
    cleanup_dart("bw-test-pub-discover");
}

#[test]
fn dart_walks_lib_skips_src() {
    let tmp = std::env::temp_dir().join("bw-test-pub-walk");
    cleanup_dart("bw-test-pub-walk");
    make_dart_fixture(&tmp, &["provider"]);

    let roots = discover_dart_externals(&tmp);
    assert_eq!(roots.len(), 1);
    let files = walk_dart_root(&roots[0]);
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(paths, vec!["ext:dart:provider/provider.dart"]);
    cleanup_dart("bw-test-pub-walk");
}

#[test]
fn parse_pubspec_lock_extracts_hosted_deps() {
    let content = r#"
packages:
  shelf:
    dependency: "direct main"
    description:
      name: shelf
      url: "https://pub.dev"
    source: hosted
    version: "1.4.1"
  flutter:
    dependency: "direct main"
    description: flutter
    source: sdk
    version: "0.0.0"
"#;
    let result = parse_pubspec_lock(content);
    let names: Vec<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"shelf"));
    assert!(!names.contains(&"flutter"));
}

#[test]
fn dart_lock_cache_fallback_finds_packages() {
    let tmp = std::env::temp_dir().join("bw-test-pub-lock-fallback");
    let cache_dir = std::env::temp_dir().join("bw-test-pub-lock-fallback-cache");
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&cache_dir);

    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("pubspec.yaml"),
        "name: app\ndependencies:\n  shelf: ^1.4.0\n"
    ).unwrap();
    let lock_content = "packages:
  shelf:
    dependency: \"direct main\"
    description:
      name: shelf
    source: hosted
    version: \"1.4.1\"
";
    // Build the cache fixture
    let hosted = cache_dir.join("hosted").join("pub.dev");
    let pkg_dir = hosted.join("shelf-1.4.1");
    let lib_dir = pkg_dir.join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(lib_dir.join("shelf.dart"), "class Shelf {}").unwrap();

    let cache_roots = vec![hosted];
    let declared = vec!["shelf".to_string()];
    let locked = parse_pubspec_lock(lock_content);
    let roots = discover_dart_externals_from_cache(&tmp, &declared, locked, &cache_roots);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "shelf");
    assert_eq!(roots[0].version, "1.4.1");

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// -----------------------------------------------------------------
// R3 — reachability-based entry resolution
// -----------------------------------------------------------------

fn mkdep(root: PathBuf, name: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: name.to_string(),
        version: String::new(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn extract_dart_exports_classifies_specifiers() {
    let src = r#"
library provider;
export 'src/provider.dart';
export 'src/value_listenable.dart' show ValueListenableBuilder;
export "package:provider/src/async.dart";
export 'package:flutter/foundation.dart'; // skipped — other package
import 'dart:async'; // skipped — dart: scheme
part 'src/part_file.dart';
        "#;
    let exports = extract_dart_exports(src, "provider");
    assert!(exports.contains(&"src/provider.dart".to_string()));
    assert!(exports.contains(&"src/value_listenable.dart".to_string()));
    assert!(exports.contains(&"src/async.dart".to_string()),
        "expected in-package spec, got {exports:?}");
    assert!(exports.contains(&"src/part_file.dart".to_string()));
    assert!(!exports.iter().any(|s| s.contains("flutter")));
}

#[test]
fn resolve_entry_follows_exports_into_src() {
    let tmp = tempfile::TempDir::new().unwrap();
    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(lib.join("src")).unwrap();
    std::fs::write(
        lib.join("provider.dart"),
        r#"
export 'src/internal.dart';
export 'public.dart';
        "#,
    ).unwrap();
    std::fs::write(lib.join("public.dart"), "class Public {}\n").unwrap();
    std::fs::write(lib.join("src").join("internal.dart"), "class Internal {}\n").unwrap();

    let dep = mkdep(lib.clone(), "provider");
    let files = PubEcosystem.resolve_import(&dep, "provider", &[]);
    assert_eq!(files.len(), 3);
    let paths: std::collections::HashSet<_> =
        files.iter().map(|f| f.absolute_path.clone()).collect();
    assert!(paths.contains(&lib.join("provider.dart")));
    assert!(paths.contains(&lib.join("public.dart")));
    assert!(paths.contains(&lib.join("src").join("internal.dart")));
    for f in &files {
        assert!(f.relative_path.starts_with("ext:dart:provider/"));
        assert_eq!(f.language, "dart");
    }
}

#[test]
fn resolve_entry_empty_without_main_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();

    let dep = mkdep(lib, "missing");
    assert!(PubEcosystem.resolve_import(&dep, "missing", &[]).is_empty());
}
