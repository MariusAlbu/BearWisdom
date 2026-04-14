// Dart / pub externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Dart pub cache → `discover_dart_externals` + `walk_dart_external_root`.
///
/// Dart packages are resolved via `.dart_tool/package_config.json`, which
/// maps each declared dependency to its on-disk root (typically
/// `~/.pub-cache/hosted/pub.dev/<name>-<version>/`). The `packageUri`
/// field (usually `lib/`) points at the public API directory.
///
/// Discovery: read `pubspec.yaml` for declared deps, then resolve each
/// through `package_config.json`. Walk: collect `lib/**/*.dart` files,
/// skipping `src/` internals (Dart convention: `lib/src/` is private).
pub struct DartExternalsLocator;

impl ExternalSourceLocator for DartExternalsLocator {
    fn ecosystem(&self) -> &'static str { "dart" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dart_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_external_root(dep)
    }
}

/// Discover external Dart package roots for a project.
///
/// Strategy:
/// 1. Read `pubspec.yaml` via the existing `PubspecManifest` reader for
///    declared dependency names.
/// 2. Parse `.dart_tool/package_config.json` (Dart 2.5+), which maps each
///    package name to its on-disk root. The `rootUri` field is either an
///    absolute `file:///` URI or a relative path from the `package_config.json`
///    directory. The `packageUri` field (typically `lib/`) is the public API root.
/// 3. For each declared dep, look up the entry in `package_config.json` and
///    resolve `rootUri + packageUri` to a concrete directory. Skip entries
///    that point back into the project (path dependencies).
pub fn discover_dart_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::pubspec::parse_pubspec_deps;

    let pubspec_path = project_root.join("pubspec.yaml");
    if !pubspec_path.is_file() {
        return Vec::new();
    }
    let Ok(pubspec_content) = std::fs::read_to_string(&pubspec_path) else {
        return Vec::new();
    };
    let declared = parse_pubspec_deps(&pubspec_content);
    if declared.is_empty() {
        return Vec::new();
    }

    let pkg_config = parse_dart_package_config(project_root);
    if pkg_config.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let project_canonical = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());
    for dep_name in &declared {
        if let Some(entry) = pkg_config.get(dep_name.as_str()) {
            let lib_dir = entry.root.join(&entry.package_uri);
            if !lib_dir.is_dir() {
                continue;
            }
            if let Ok(canonical) = lib_dir.canonicalize() {
                if canonical.starts_with(&project_canonical) {
                    continue;
                }
            }
            result.push(ExternalDepRoot {
                module_path: dep_name.clone(),
                version: entry.version.clone(),
                root: lib_dir,
                ecosystem: "dart",
                package_id: None,
            });
        }
    }
    debug!("Dart: discovered {} external package roots", result.len());
    result
}

struct DartPackageEntry {
    root: PathBuf,
    package_uri: String,
    version: String,
}

fn parse_dart_package_config(project_root: &Path) -> std::collections::HashMap<String, DartPackageEntry> {
    let config_path = project_root.join(".dart_tool").join("package_config.json");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return std::collections::HashMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return std::collections::HashMap::new();
    };
    let Some(packages) = json.get("packages").and_then(|v| v.as_array()) else {
        return std::collections::HashMap::new();
    };

    let config_dir = config_path.parent().unwrap_or(project_root);
    let mut map = std::collections::HashMap::new();

    for pkg in packages {
        let Some(name) = pkg.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(root_uri) = pkg.get("rootUri").and_then(|v| v.as_str()) else {
            continue;
        };
        let package_uri = pkg
            .get("packageUri")
            .and_then(|v| v.as_str())
            .unwrap_or("lib/")
            .to_string();

        let root = if root_uri.starts_with("file:///") {
            PathBuf::from(&root_uri[7..].replace('/', std::path::MAIN_SEPARATOR_STR))
        } else if root_uri.starts_with("file://") {
            PathBuf::from(&root_uri[7..].replace('/', std::path::MAIN_SEPARATOR_STR))
        } else {
            config_dir.join(root_uri.replace('/', std::path::MAIN_SEPARATOR_STR))
        };

        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        map.insert(name.to_string(), DartPackageEntry {
            root,
            package_uri,
            version,
        });
    }
    map
}

/// Walk a Dart package's public API directory (`lib/`).
///
/// Collects `*.dart` files, skipping `src/` (Dart convention: `lib/src/`
/// is private implementation, `lib/*.dart` is the public API). Also skips
/// test, build, and hidden directories.
pub fn walk_dart_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dart_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_dart_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_dart_dir_bounded(dir, root, dep, out, 0);
}

fn walk_dart_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "src" | "test" | "tests" | "example" | "build" | "doc"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_dart_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".dart") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:dart:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "dart",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let tmp = std::env::temp_dir().join("bw-test-dart-discover");
        cleanup_dart("bw-test-dart-discover");
        make_dart_fixture(&tmp, &["http", "provider"]);

        let roots = discover_dart_externals(&tmp);
        let mut names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["http", "provider"]);
        assert_eq!(roots[0].ecosystem, "dart");

        cleanup_dart("bw-test-dart-discover");
    }

    #[test]
    fn dart_walks_lib_skips_src() {
        let tmp = std::env::temp_dir().join("bw-test-dart-walk");
        cleanup_dart("bw-test-dart-walk");
        make_dart_fixture(&tmp, &["provider"]);

        let roots = discover_dart_externals(&tmp);
        assert_eq!(roots.len(), 1);

        let files = walk_dart_external_root(&roots[0]);
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert_eq!(paths, vec!["ext:dart:provider/provider.dart"]);

        cleanup_dart("bw-test-dart-walk");
    }

    #[test]
    fn dart_skips_undeclared_packages() {
        let tmp = std::env::temp_dir().join("bw-test-dart-undeclared");
        cleanup_dart("bw-test-dart-undeclared");
        make_dart_fixture(&tmp, &["http"]);
        let dart_tool = tmp.join(".dart_tool");
        let config_str = std::fs::read_to_string(dart_tool.join("package_config.json")).unwrap();
        let mut config: serde_json::Value = serde_json::from_str(&config_str).unwrap();
        config["packages"].as_array_mut().unwrap().push(serde_json::json!({
            "name": "rogue_pkg",
            "rootUri": "../../_dart_pub_cache/rogue_pkg-0.1.0",
            "packageUri": "lib/",
            "version": "0.1.0"
        }));
        std::fs::write(dart_tool.join("package_config.json"), config.to_string()).unwrap();

        let roots = discover_dart_externals(&tmp);
        let names: Vec<String> = roots.iter().map(|r| r.module_path.clone()).collect();
        assert_eq!(names, vec!["http".to_string()]);

        cleanup_dart("bw-test-dart-undeclared");
    }

    #[test]
    fn dart_empty_without_package_config() {
        let tmp = std::env::temp_dir().join("bw-test-dart-no-config");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("pubspec.yaml"), "name: app\ndependencies:\n  http: ^1.0.0\n").unwrap();

        let roots = discover_dart_externals(&tmp);
        assert!(roots.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
